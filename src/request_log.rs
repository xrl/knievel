//! Per-request structured logging middleware (Phase 4.12).
//!
//! Emits one structured tracing event per completed HTTP request,
//! regardless of status. The shape is deliberately spartan:
//!
//! - `method`        — HTTP method
//! - `path`          — URI path (no query string, no matched route);
//!                     see "Choices" below
//! - `status`        — final HTTP status code
//! - `latency_ms`    — wall-clock from middleware-enter to
//!                     middleware-exit, integer milliseconds
//! - `request_id`    — minted at middleware-enter (32 hex chars from
//!                     OsRng) unless an inbound `x-request-id` is
//!                     present and looks safe to honor
//!
//! For 5xx the line is at `tracing::error!` level; for slow
//! requests crossing `slow_ms` it bumps to `tracing::warn!`. Every
//! response carries the `x-request-id` response header so clients
//! can correlate failures with server-side log lines.
//!
//! ## Choices
//!
//! 1. **URI path, not matched route.** Poem's `Endpoint` trait
//!    doesn't expose the matched route in a stable way that's
//!    available *after* the inner endpoint resolves. URI path is
//!    concrete, always available, and what an operator already
//!    sees in nginx access logs.
//!
//! 2. **No query string.** The cost of a query-key allow-list
//!    getting it wrong (logging a `?token=…` value) is much higher
//!    than the cost of operators losing query visibility — most
//!    knievel parameters are paginated `limit`/`cursor` /
//!    `external_id`, none of which we need in the log line.
//!
//! 3. **No body, no Authorization header, no Cookie header.**
//!    Bodies may contain Liquid templates / customer secrets / PII;
//!    Authorization is a bearer; Cookie carries the same thing for
//!    PKCE flows. The presence-or-absence of a bearer is implicit
//!    in the bearer-rejection log line at `src/auth/security.rs`.
//!
//! 4. **No principal `actor_id` at info level.** The principal's
//!    `org_id` and `role` are useful for "this user can't see X"
//!    debugging; `actor_id` (= the token row id or the JWT subject)
//!    is one bit closer to PII. Today the middleware can't see the
//!    principal at all — `BearerAuth` runs *inside* the inner
//!    endpoint — so this is moot, but the rule is documented here
//!    for the day a `tracing::Span` field gets populated from a
//!    handler.
//!
//! 5. **Hot-path consideration.** The decision endpoint
//!    (`POST /v1/projects/:project_id/decisions`) is the highest-
//!    throughput surface. Trust `logging.request_log_skip_paths`
//!    to let operators turn it off there if needed; v0 doesn't
//!    grow a sampling system. Operators with tight budgets add
//!    the path to the skip list. (`logging.decisions_sample_rate`
//!    in the chart is for a different sampler that doesn't read
//!    this middleware.)
//!
//! 6. **`x-request-id` is honored when present.** Inbound IDs that
//!    look like a printable, length-bounded ASCII string are
//!    accepted verbatim; anything else (control characters, > 128
//!    chars, < 8 chars) is replaced with a freshly minted ID. The
//!    response always carries the *effective* ID.

use std::sync::Arc;
use std::time::Instant;

use argon2::password_hash::rand_core::{OsRng, RngCore};
use poem::http::{HeaderName, HeaderValue, StatusCode};
use poem::{Endpoint, IntoResponse, Middleware, Request, Response, Result};

const HEADER: &str = "x-request-id";

/// Configuration handed to the middleware at install time.
#[derive(Clone, Debug)]
pub struct RequestLogConfig {
    /// Skip-list of exact-match paths whose requests are NOT
    /// logged. The standard probe paths (`/healthz`, `/readyz`)
    /// are the default; operators add hot endpoints when needed.
    pub skip_paths: Arc<Vec<String>>,
    /// Latency threshold above which a request is logged at
    /// `warn!` regardless of status. Helps surface slow-but-200
    /// paths.
    pub slow_ms: u64,
}

impl Default for RequestLogConfig {
    fn default() -> Self {
        Self {
            skip_paths: Arc::new(vec!["/healthz".into(), "/readyz".into()]),
            slow_ms: 1000,
        }
    }
}

/// Middleware factory.
#[derive(Clone, Debug, Default)]
pub struct RequestLog {
    cfg: RequestLogConfig,
}

impl RequestLog {
    pub fn new(cfg: RequestLogConfig) -> Self {
        Self { cfg }
    }
}

impl<E: Endpoint> Middleware<E> for RequestLog {
    type Output = RequestLogEndpoint<E>;
    fn transform(&self, inner: E) -> Self::Output {
        RequestLogEndpoint {
            inner,
            cfg: self.cfg.clone(),
        }
    }
}

pub struct RequestLogEndpoint<E> {
    inner: E,
    cfg: RequestLogConfig,
}

impl<E: Endpoint> Endpoint for RequestLogEndpoint<E> {
    type Output = Response;

    async fn call(&self, mut req: Request) -> Result<Self::Output> {
        let started = Instant::now();
        let method = req.method().clone();
        let path = req.uri().path().to_string();

        // Honor inbound x-request-id if present and safe; mint
        // otherwise. We never blindly forward an inbound value —
        // tag it onto the response only after the validation
        // below.
        let request_id = req
            .header(HEADER)
            .and_then(safe_request_id)
            .unwrap_or_else(mint_request_id);

        // Stash on the request so handlers can read it via
        // `req.data::<RequestId>()` if they want to (none today).
        req.set_data(RequestId(request_id.clone()));

        // Skip-list: emit no log line, but still attach the
        // response header so correlation is possible if the load
        // balancer is logging the header. Probe traffic (k8s
        // /healthz / /readyz) is the canonical reason to skip.
        let skip = self.cfg.skip_paths.iter().any(|p| p == &path);

        let result = self.inner.call(req).await;
        let latency_ms = started.elapsed().as_millis() as u64;

        match result {
            Ok(out) => {
                let mut response = out.into_response();
                if !skip {
                    log_event(
                        response.status(),
                        latency_ms,
                        &method,
                        &path,
                        &request_id,
                        None,
                        self.cfg.slow_ms,
                    );
                }
                set_request_id_header(&mut response, &request_id);
                Ok(response)
            }
            Err(err) => {
                let status = err.status();
                let err_str = format!("{err}");
                let mut r = err.into_response();
                if !skip {
                    log_event(
                        status,
                        latency_ms,
                        &method,
                        &path,
                        &request_id,
                        Some(&err_str),
                        self.cfg.slow_ms,
                    );
                }
                set_request_id_header(&mut r, &request_id);
                Ok(r)
            }
        }
    }
}

/// Extension stashed on the `Request` so handlers / extractors
/// can pull the request_id without re-parsing the header. Held
/// behind a newtype so `Request::data::<RequestId>()` is
/// unambiguous.
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn set_request_id_header(resp: &mut Response, id: &str) {
    if let Ok(value) = HeaderValue::from_str(id) {
        resp.headers_mut()
            .insert(HeaderName::from_static(HEADER), value);
    }
}

fn safe_request_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.len() < 8 || trimmed.len() > 128 {
        return None;
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_graphic() || c == '-' || c == '_')
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn mint_request_id() -> String {
    // 16 random bytes → 32 lowercase hex chars. Argon2 is
    // already a dependency, so we lift its OsRng rather than
    // adding a separate `rand` import.
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    let mut s = String::with_capacity(32);
    for b in &bytes {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

fn log_event(
    status: StatusCode,
    latency_ms: u64,
    method: &poem::http::Method,
    path: &str,
    request_id: &str,
    err_chain: Option<&str>,
    slow_ms: u64,
) {
    // 5xx → error, slow-but-otherwise-fine → warn, 4xx and 2xx /
    // 3xx → info. The shape (same field set) is identical at
    // every level so operators get one Loki/Splunk query that
    // works across status classes.
    let s = status.as_u16();
    let slow = latency_ms >= slow_ms;
    if s >= 500 {
        tracing::error!(
            method = %method,
            path = %path,
            status = s,
            latency_ms = latency_ms,
            request_id = %request_id,
            error_chain = err_chain.unwrap_or(""),
            "request"
        );
    } else if slow {
        tracing::warn!(
            method = %method,
            path = %path,
            status = s,
            latency_ms = latency_ms,
            request_id = %request_id,
            slow = true,
            "request slow"
        );
    } else {
        tracing::info!(
            method = %method,
            path = %path,
            status = s,
            latency_ms = latency_ms,
            request_id = %request_id,
            "request"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poem::test::TestClient;
    use poem::{handler, EndpointExt, Route};
    use std::sync::Mutex;
    use tracing::Subscriber;
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::registry::LookupSpan;
    use tracing_subscriber::Layer;

    /// Tracing layer that captures rendered events into an
    /// in-memory buffer the tests can inspect. Picked over
    /// `tracing-test` to avoid adding a dev-dep purely for two
    /// assertions; the layer is small enough to inline.
    #[derive(Clone, Default)]
    struct CaptureLayer {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl<S> Layer<S> for CaptureLayer
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = StringVisitor::default();
            event.record(&mut visitor);
            let level = event.metadata().level().to_string();
            self.events
                .lock()
                .unwrap()
                .push(format!("{} {}", level, visitor.0));
        }
    }

    #[derive(Default)]
    struct StringVisitor(String);
    impl tracing::field::Visit for StringVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            use std::fmt::Write as _;
            let _ = write!(self.0, " {}={:?}", field.name(), value);
        }
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            use std::fmt::Write as _;
            let _ = write!(self.0, " {}={}", field.name(), value);
        }
        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            use std::fmt::Write as _;
            let _ = write!(self.0, " {}={}", field.name(), value);
        }
        fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
            use std::fmt::Write as _;
            let _ = write!(self.0, " {}={}", field.name(), value);
        }
        fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
            use std::fmt::Write as _;
            let _ = write!(self.0, " {}={}", field.name(), value);
        }
    }

    fn install_capture() -> (tracing::dispatcher::DefaultGuard, Arc<Mutex<Vec<String>>>) {
        let layer = CaptureLayer::default();
        let buf = layer.events.clone();
        let subscriber = tracing_subscriber::registry().with(layer);
        let guard = tracing::subscriber::set_default(subscriber);
        (guard, buf)
    }

    #[handler]
    fn ok_handler() -> &'static str {
        "ok"
    }

    #[handler]
    fn boom_handler() -> Response {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body("boom")
    }

    #[handler]
    fn forbidden_handler() -> Response {
        Response::builder().status(StatusCode::FORBIDDEN).body("nope")
    }

    #[handler]
    async fn slow_handler() -> &'static str {
        // ~25ms — well over the test slow_ms threshold (10).
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        "slow-ok"
    }

    fn app() -> impl Endpoint {
        let cfg = RequestLogConfig {
            skip_paths: Arc::new(vec!["/healthz".into()]),
            slow_ms: 10,
        };
        Route::new()
            .at("/ok", poem::get(ok_handler))
            .at("/boom", poem::get(boom_handler))
            .at("/forbidden", poem::get(forbidden_handler))
            .at("/slow", poem::get(slow_handler))
            .at("/healthz", poem::get(ok_handler))
            .with(RequestLog::new(cfg))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ok_emits_info_with_request_id_and_status_200() {
        let (_g, events) = install_capture();
        let cli = TestClient::new(app());
        let resp = cli.get("/ok").send().await;
        resp.assert_status_is_ok();
        assert!(resp.0.headers().get(HEADER).is_some());
        let events = events.lock().unwrap().clone();
        let line = events
            .iter()
            .find(|e| e.contains("status=200") && e.contains("path=/ok"))
            .unwrap_or_else(|| panic!("no 200 line in {events:?}"));
        assert!(line.starts_with("INFO"), "expected INFO, got: {line}");
        assert!(line.contains("method=GET"));
        assert!(line.contains("request_id="));
        assert!(line.contains("latency_ms="));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn five_hundred_emits_error_level() {
        let (_g, events) = install_capture();
        let cli = TestClient::new(app());
        let resp = cli.get("/boom").send().await;
        assert_eq!(resp.0.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(resp.0.headers().get(HEADER).is_some());
        let events = events.lock().unwrap().clone();
        let line = events
            .iter()
            .find(|e| e.contains("status=500"))
            .unwrap_or_else(|| panic!("no 500 line in {events:?}"));
        assert!(line.starts_with("ERROR"), "expected ERROR, got: {line}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn four_hundred_emits_info_level() {
        let (_g, events) = install_capture();
        let cli = TestClient::new(app());
        let resp = cli.get("/forbidden").send().await;
        assert_eq!(resp.0.status(), StatusCode::FORBIDDEN);
        let events = events.lock().unwrap().clone();
        let line = events
            .iter()
            .find(|e| e.contains("status=403"))
            .unwrap_or_else(|| panic!("no 403 line in {events:?}"));
        assert!(line.starts_with("INFO"), "expected INFO, got: {line}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn slow_ok_emits_warn_level() {
        let (_g, events) = install_capture();
        let cli = TestClient::new(app());
        let resp = cli.get("/slow").send().await;
        resp.assert_status_is_ok();
        let events = events.lock().unwrap().clone();
        let line = events
            .iter()
            .find(|e| e.contains("path=/slow"))
            .unwrap_or_else(|| panic!("no /slow line in {events:?}"));
        assert!(
            line.starts_with("WARN"),
            "slow request should warn: {line}"
        );
        assert!(line.contains("slow=true"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn skipped_path_emits_no_log_but_still_sets_header() {
        let (_g, events) = install_capture();
        let cli = TestClient::new(app());
        let resp = cli.get("/healthz").send().await;
        resp.assert_status_is_ok();
        assert!(resp.0.headers().get(HEADER).is_some());
        let events = events.lock().unwrap().clone();
        assert!(
            events.iter().all(|e| !e.contains("path=/healthz")),
            "expected no log line for /healthz, got: {events:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn inbound_request_id_is_honored_when_safe() {
        let (_g, events) = install_capture();
        let cli = TestClient::new(app());
        let resp = cli
            .get("/ok")
            .header(HEADER, "abc-1234-test-id")
            .send()
            .await;
        resp.assert_status_is_ok();
        assert_eq!(
            resp.0.headers().get(HEADER).unwrap().to_str().unwrap(),
            "abc-1234-test-id"
        );
        let events = events.lock().unwrap().clone();
        assert!(events
            .iter()
            .any(|e| e.contains("request_id=abc-1234-test-id")));
    }

    #[test]
    fn safe_request_id_accepts_uuid() {
        assert!(safe_request_id("550e8400-e29b-41d4-a716-446655440000").is_some());
    }
    #[test]
    fn safe_request_id_rejects_short() {
        assert!(safe_request_id("abc").is_none());
    }
    #[test]
    fn safe_request_id_rejects_long() {
        let long = "a".repeat(200);
        assert!(safe_request_id(&long).is_none());
    }
    #[test]
    fn safe_request_id_rejects_control_chars() {
        assert!(safe_request_id("hello\nworld-12345").is_none());
    }
    #[test]
    fn mint_request_id_is_32_hex() {
        let id = mint_request_id();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
