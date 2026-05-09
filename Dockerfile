# syntax=docker/dockerfile:1.7
#
# Knievel container image — runtime-only.
#
# This Dockerfile does NOT compile Rust. The cargo build runs as
# a bare step in the surrounding driver (release.yml's
# publish-image matrix in CI; `cargo xtask build-image` locally)
# so the build is debuggable, the cache is plain Swatinem
# rust-cache (or your local `~/.cargo` + `target/`), and there's
# no QEMU emulation cost on cross-platform releases. The
# Dockerfile just packages whatever binaries you've already
# produced into a distroless runtime.
#
# Build context expects, at the root:
#   ./knievel                  (server binary, target arch)
#   ./knievel-cli              (CLI binary, target arch)
#   ./web/admin/dist/          (admin SPA bundle)
#
# `cargo xtask build-image` stages this layout for you. The CI
# `publish-image` job stages it inline.
#
# Runtime base: distroless `cc:nonroot`. `cc` (rather than
# `static`) because rustls-tls links libgcc_s. `nonroot` provides
# UID 65532 with no shell, no package manager, and no /etc/passwd
# that an attacker can append to.

FROM gcr.io/distroless/cc-debian12:nonroot

COPY knievel     /usr/local/bin/knievel
COPY knievel-cli /usr/local/bin/knievel-cli
COPY web/admin/dist /var/lib/knievel/admin

USER nonroot:nonroot
EXPOSE 8080

# Inputs the runtime needs:
#   - /etc/knievel/config.yaml (mounted via volume in compose,
#     ConfigMap in Helm). Layered loader picks up KNIEVEL_*
#     env vars on top.
ENV KNIEVEL_CONFIG=/etc/knievel/config.yaml

# Admin UI mount point. Set to empty string to disable
# (the StaticFilesEndpoint isn't installed when this is empty,
# so `/admin/*` returns 404 — same image runs as a headless API).
ENV KNIEVEL_ADMIN_UI__STATIC_DIR=/var/lib/knievel/admin

ENTRYPOINT ["/usr/local/bin/knievel"]
