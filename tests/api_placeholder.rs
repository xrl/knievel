//! Placeholder for the api / contract test slice.
//!
//! API-level integration tests live in `tests/api_*.rs` files.
//! Phase 3 lands the first real ones; this placeholder ensures
//! the nextest filter `binary(/^api/)` has at least one binary to
//! match during Phase 1–2, so the `api-contract` CI job parses.
//!
//! `--no-tests=pass` plus zero `#[test]` functions in this binary
//! means the job reports success once the filter resolves.
