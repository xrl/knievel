# Release Checklist

This checklist gates every `v*` tag on the platform repo. It's
enforced by CI as a required PR comment on the release-tagging PR
(per `TESTING.md` § 10.3) and by the maintainer reviewing the tag.

The form below is intended to be **pasted into the release PR**.
Each item is a checkbox; skipping any item requires a brief written
justification in the same PR.

---

## Release Checklist for `vX.Y.Z`

**Releaser:** _@github-handle_
**Target tag:** `vX.Y.Z`
**Previous tag:** `vA.B.C`
**Release notes anchor:** `CHANGELOG.md` § `[X.Y.Z]`

### Tests + gates

- [ ] CI green on the release PR (full per-PR DAG —
      `.github/workflows/ci.yml`).
- [ ] **Cross-tenant integration tests pass** — `cargo xtask
      check-cross-tenant` reports 100% endpoint coverage; every
      `tests/api_*.rs` cross-tenant case is green.
- [ ] **Migration linter passes** — `cargo xtask lint-migrations`
      reports clean.
- [ ] OpenAPI drift gate green — `cargo xtask openapi --check`.
- [ ] Doc-table gate green — `cargo xtask check-api-doc` (every
      endpoint in the spec is documented in `API.md`).
- [ ] Doc-fence gate green — `cargo xtask check-doc-fences` (every
      fenced code block parses).
- [ ] Link-checker green — `lychee` against intra-repo `.md` paths.
- [ ] Acceptance suite (`tests/acceptance.rs` ACC-01..14) green
      against the compose stack.

### Manual review

- [ ] **Auth-config or RLS-policy changes** since the previous tag
      were reviewed by a second maintainer. `git diff vA.B.C..HEAD
      -- migrations/ src/auth/ src/handlers.rs src/db.rs`.
- [ ] **No new endpoints accept tenant identity from the request
      body** — only path-derived or token-derived. (Auditor: scan
      handlers added since the previous tag for `org_id` /
      `project_id` in request types.)
- [ ] **No new logging surfaces PII** — no raw user-agent strings,
      no IP addresses outside `events_raw`, no JWT contents,
      no full bearer tokens. (Auditor: scan
      `src/observability.rs` and any new `tracing::` call sites.)
- [ ] **Schema migrations are additive** — `git diff vA.B.C..HEAD
      -- migrations/` shows only `CREATE TABLE`, `ALTER TABLE …
      ADD COLUMN`, `CREATE INDEX`. No `DROP COLUMN`, no `DROP
      TABLE` (a column marked deprecated for ≥ 6 months may drop
      with a justification linking the deprecation commit).
- [ ] **CLAUDE.md "Open known gaps"** has been re-checked; any
      gap that closed in this release moves to a CHANGELOG entry.

### CHANGELOG + versioning

- [ ] `CHANGELOG.md` `[Unreleased]` section is moved to `[X.Y.Z]`
      and includes:
  - Added: every user-facing addition.
  - Changed: every wire-format-affecting change (with the
    `REQUIREMENTS.md § 6.4` justification when the change is a
    deprecation step).
  - Fixed: every bug fix.
  - Security: every advisory CVE / GHSA, with credit.
- [ ] Compatibility matrix is honored:
  - **Patch (`X.Y.Z` → `X.Y.(Z+1)`):** no spec changes; bug fixes
    + doc fixes only. Generated gem is regenerated only if the
    spec materially changed.
  - **Minor (`X.Y.0` → `X.(Y+1).0`):** spec is additive; gem
    `X.(Y+1).*` mirrors. Old gem `X.Y.*` continues to work.
  - **Major (`X.0.0` → `(X+1).0.0`):** breaking change permitted
    only after the 6-month deprecation window.
- [ ] `Cargo.toml` workspace `version` matches the target tag.

### Release artifacts

- [ ] `release.yml` workflow ran successfully on the tag (all jobs
      green: `publish-image` (matrix: amd64 + arm64),
      `publish-image-merge`, `build-cli`, `github-release`,
      `release-ruby-gem`). The PR that merged the release commit
      to `main` already ran the per-PR DAG; the tag workflow does
      not re-run it (TESTING.md § 12.9).
- [ ] Container image present: `ghcr.io/knievel-ads/knievel:vX.Y.Z`.
      Multi-arch (`amd64` + `arm64`).
- [ ] Image cosigned + provenance-attested. `cosign verify
      ghcr.io/knievel-ads/knievel@<digest> \
      --certificate-identity-regexp '…' \
      --certificate-oidc-issuer-regexp 'https://token.actions.githubusercontent.com'`.
- [ ] CLI binaries attached to the GitHub Release for all four
      target triples
      (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`,
      `x86_64-apple-darwin`, `aarch64-apple-darwin`), each with a
      `.sha256` sidecar and a cosign sign-blob bundle.
- [ ] `release-ruby-gem.yml` workflow ran successfully — gem
      tagged on `knievel-ruby` and published to RubyGems.
- [ ] `gem install knievel -v X.Y.Z` succeeds from a clean machine
      and `require 'knievel'` works.

### Operator-facing

- [ ] `DEPLOYMENT.md` § "Sizing guidance" still reflects reality
      (or the v0 "TARGET (unverified)" caveat is still honest).
- [ ] `MIGRATION_RX.md` (and any other consumer migration files)
      updated for any wire-format changes.
- [ ] If this release contains a deprecation, it ships with the
      6-month sunset header (`Deprecation: true`,
      `Sunset: <RFC-3339-date>`) and a CHANGELOG note.

### Sign-off

- [ ] Releaser: _@github-handle_, _date_
- [ ] Second-maintainer review on auth/RLS changes (or N/A if no
      such changes): _@github-handle_, _date_

---

If any item is **N/A**, replace the checkbox with `~` strikethrough
and add a one-line note on why the item didn't apply.

If any item is **skipped with justification**, leave the box
unchecked and add a sub-bullet explaining why. The maintainer
reviewing the tag is the final authority on whether the
justification is acceptable.
