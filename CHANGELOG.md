# Changelog

All notable changes to knievel are documented in this file. Format
follows [keep-a-changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows the additive-forever compatibility policy in
`REQUIREMENTS.md` § 6.4.

This file is generated from each release-tagging PR's "release
notes" section. It is **not** hand-edited per commit; the
authoritative per-commit log is `git log` plus `PHASES.md`.

## [Unreleased]

### Added

(none)

### Changed

(none)

### Fixed

(none)

## [0.1.13] — 2026-05-07

### Added

(none)

### Changed

(none)

### Fixed

(none)

## [0.1.12] — 2026-05-07

### Added

(none)

### Changed

(none)

### Fixed

(none)

## [0.1.11] — 2026-05-07

### Added

(none)

### Changed

(none)

### Fixed

(none)

## [0.1.10] — 2026-05-07

### Added

(none)

### Changed

(none)

### Fixed

(none)

## [0.1.9] — 2026-05-07

### Added

(none)

### Changed

(none)

### Fixed

(none)

## [0.1.8] — 2026-05-07

### Added

(none)

### Changed

(none)

### Fixed

(none)

## [0.1.7] — 2026-05-07

### Added

- Phase 5 documentation set: `README.md` (`5.1`), `ARCHITECTURE.md`
  (`5.2`), `DEPLOYMENT.md` (`5.3`), `CONTRIBUTING.md` /
  `SECURITY.md` / `CHANGELOG.md` (`5.4`), `RELEASE_CHECKLIST.md` /
  `RELEASE_PLAYBOOK.md` (`5.5`).
- `xtask check-doc-fences`, `xtask check-api-doc`, lychee link
  checking in CI (`5.6`).
- First benchmark run + `bench/results/v0.1.md` (`5.7`).

### Changed

(none)

### Fixed

(none)

## [0.1.6] — 2026-05-06

### Added

- **Phase 4.10:** End-to-end gem smoke against the compose stack as
  a step in `release-ruby-gem.yml`. Closes
  `REQUIREMENTS.md § 8` item 3 ("a third party can integrate from
  the gem alone"). Local equivalent: `docker compose up && ruby
  examples/compose/gem_smoke.rb`. Phase 4.10 marked `[x]`.

## [0.1.5] — 2026-05-06

### Added

- **Phase 4.10 (partial):** Hand-written `Enumerable` wrapper layer
  in `knievel-ruby` — `Knievel::Resources::*` (one per paginated
  resource) and `Knievel::Client` (URL-parsing facade). 24 rspec
  examples cover cursor walks, `lazy.first(n)` short-circuit,
  filter forwarding, page-size validation.
- `.openapi-generator-ignore` extended (canonical version in
  `.github/ruby-client/`) to protect wrapper paths through
  regeneration.

## [0.1.4] — 2026-05-06

### Added

- **Phase 3.33:** Server-side cursor pagination on the eight
  demand+inventory list endpoints (`advertisers`, `campaigns`,
  `flights`, `ads`, `creatives`, `creative_templates`, `sites`,
  `zones`). `?limit=N&cursor=<opaque>` per `API.md` § "Pagination";
  default 50, max 500.
- `src/pagination.rs` core: `base64url(JSON{kind, last_id})`
  cursor with kind validation (cross-resource replay → `400
  invalid_cursor`); `?limit=0` and `?limit > 500` → `400
  invalid_limit`. 13 unit tests + 7 API-level tests.
- `400 BadRequest` variant added to each `List*Resp` ApiResponse
  enum on the affected resources.

### Changed

- Three taxonomy list endpoints (`listChannels`, `listPriorities`,
  `listAdTypes`) and two TEXT-id list endpoints
  (`listAdLibraryItems`, `listTokens`) remain non-paginated for
  v0; documented in API.md and deferred to Phase 6.5.
- The `x-knievel-paginated*` vendor extensions API.md aspirationally
  promised are deferred to Phase 6.6 — poem-openapi 5 has no
  operation-level extension API; we'll upstream it rather than
  carrying a `cargo xtask openapi` post-processor.

## [0.1.3] — 2026-05-06

### Added

- **Phase 4.10 (partial):** Default `servers:` block stamped into
  `openapi.yaml` (`http://localhost:8080`) so the generated Ruby
  gem doesn't default to `http://localhost`. Both the static spec
  (`lib.rs::openapi_spec_yaml()`) and the live spec
  (`server.rs::routes()`) read from a shared
  `DEFAULT_OPENAPI_SERVER_URL` constant.

## [0.1.2] — 2026-05-06

### Added

- **Phase 4.10 (partial):** Operation tagging via
  `src/api_tags.rs` and `#[OpenApi(tag = "ApiTags::…")]` on the 15
  resource modules. The Ruby gem now exposes 15 focused API classes
  (`Knievel::AdvertisersApi`, `Knievel::CampaignsApi`, …) instead
  of one 3970-line `DefaultApi`. Variant doc-comments flow through
  to tag descriptions in the spec.

## [0.1.1] — 2026-05-06

### Added

- **Phase 4.10 (partial):** Generator CI for the Ruby gem.
  `.github/workflows/release-ruby-gem.yml` triggers on `v*` tags,
  mints an installation token via the `knievel-pipelines` GitHub
  App, regenerates the Faraday-based gem from `openapi.yaml`,
  smoke-tests the build, and commits + tags
  `knievel-ads/knievel-ruby` with the matching version.
  `.github/workflows/publish-rubygems.yml` (in knievel-ruby) takes
  the new tag and `gem push`es to RubyGems via `RUBYGEMS_ORG_API_KEY`.

## [0.1.0] — squat tag

Squatted `knievel` on RubyGems. No public release; first real
release was `0.1.1`.

[Unreleased]: https://github.com/knievel-ads/knievel/compare/v0.1.13...HEAD
[0.1.13]: https://github.com/knievel-ads/knievel/compare/v0.1.12...v0.1.13
[0.1.12]: https://github.com/knievel-ads/knievel/compare/v0.1.11...v0.1.12
[0.1.11]: https://github.com/knievel-ads/knievel/compare/v0.1.10...v0.1.11
[0.1.10]: https://github.com/knievel-ads/knievel/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/knievel-ads/knievel/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/knievel-ads/knievel/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/knievel-ads/knievel/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/knievel-ads/knievel/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/knievel-ads/knievel/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/knievel-ads/knievel/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/knievel-ads/knievel/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/knievel-ads/knievel/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/knievel-ads/knievel/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/knievel-ads/knievel/releases/tag/v0.1.0
