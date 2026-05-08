# syntax=docker/dockerfile:1.7
#
# Knievel container image.
#
# Phase 4.1 ships this Dockerfile alongside the compose stack so a
# contributor can `docker compose up` against a locally-built image.
# Phase 4.3 wires GitHub Actions to build and publish multi-arch
# variants to `ghcr.io/xrl/knievel`; the same Dockerfile drives both
# paths.
#
# Stage 1 — build with the standard rust toolchain image. We use
#   bookworm rather than alpine because sqlx + rustls pull in
#   crates that link against system OpenSSL on alpine without
#   feature gymnastics. The release binary is what gets copied
#   forward; the builder image stays in the build cache.
#
# Stage 2 — distroless `cc:nonroot`. `cc` (rather than `static`)
#   is required because rustls-tls links libgcc_s. `nonroot`
#   provides UID 65532 with no shell, no package manager, and no
#   /etc/passwd that an attacker can append to.

FROM rust:1-bookworm AS builder

WORKDIR /build

# Cache the dependency graph independently of the source: a stub
# crate skeleton is enough to populate the registry and download
# everything cargo needs. Real source comes in afterwards so a
# code edit doesn't bust the dep cache.
COPY Cargo.toml Cargo.lock ./
COPY xtask/Cargo.toml xtask/Cargo.toml
COPY testlib/Cargo.toml testlib/Cargo.toml
RUN mkdir -p src/bin xtask/src testlib/src benches \
 && echo 'fn main() {}' > src/main.rs \
 && echo '' > src/lib.rs \
 && echo 'fn main() {}' > src/bin/knievel_cli.rs \
 && echo 'fn main() {}' > xtask/src/main.rs \
 && echo '' > testlib/src/lib.rs \
 && echo 'fn main() {}' > benches/selection_pick.rs \
 && echo 'fn main() {}' > benches/hmac_verify.rs \
 && echo 'fn main() {}' > benches/decision_handler.rs \
 && echo 'fn main() {}' > benches/iai_decision.rs \
 && echo 'fn main() {}' > benches/iai_hmac.rs \
 && echo 'fn main() {}' > benches/dhat_decision.rs \
 && cargo build --release --locked --bins \
 && rm -rf src xtask/src testlib/src benches

# Real source.
COPY build.rs ./build.rs
COPY migrations migrations
COPY src src
COPY xtask/src xtask/src
COPY testlib/src testlib/src
COPY benches benches
RUN cargo build --release --locked --bin knievel --bin knievel-cli \
 && strip target/release/knievel target/release/knievel-cli

FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /build/target/release/knievel     /usr/local/bin/knievel
COPY --from=builder /build/target/release/knievel-cli /usr/local/bin/knievel-cli

# Admin UI bundle (Phase 7.11). Pre-built in CI / locally via
# `cargo xtask build-image` (see UI.md "Deployment /
# Single-image Dockerfile"). The Node build runs OUTSIDE the
# container so pnpm's store cache works natively, and the
# Dockerfile stays free of a Node toolchain. The build
# context must contain `web/admin/dist/`; for headless API
# builds, the directory can be empty — the runtime mount is
# gated separately by `KNIEVEL_ADMIN_UI__STATIC_DIR` below.
COPY web/admin/dist /var/lib/knievel/admin

USER nonroot:nonroot
EXPOSE 8080

# Inputs the runtime needs:
#   - /etc/knievel/config.yaml (mounted via volume in compose,
#     ConfigMap in Helm). Layered loader picks up KNIEVEL_*
#     env vars on top.
ENV KNIEVEL_CONFIG=/etc/knievel/config.yaml

# Admin UI mount point. Set to empty string to disable
# (the StaticFilesEndpoint isn't installed when this is
# empty, so `/admin/*` returns 404 — same image runs as a
# headless API).
ENV KNIEVEL_ADMIN_UI__STATIC_DIR=/var/lib/knievel/admin

ENTRYPOINT ["/usr/local/bin/knievel"]
