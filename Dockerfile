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
 && cargo build --release --locked --bins \
 && rm -rf src xtask/src testlib/src benches

# Real source.
COPY build.rs ./build.rs
COPY migrations migrations
COPY src src
COPY xtask/src xtask/src
COPY testlib/src testlib/src
RUN cargo build --release --locked --bin knievel --bin knievel-cli \
 && strip target/release/knievel target/release/knievel-cli

FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /build/target/release/knievel     /usr/local/bin/knievel
COPY --from=builder /build/target/release/knievel-cli /usr/local/bin/knievel-cli

USER nonroot:nonroot
EXPOSE 8080

# Inputs the runtime needs:
#   - /etc/knievel/config.yaml (mounted via volume in compose,
#     ConfigMap in Helm). Layered loader picks up KNIEVEL_*
#     env vars on top.
ENV KNIEVEL_CONFIG=/etc/knievel/config.yaml

ENTRYPOINT ["/usr/local/bin/knievel"]
