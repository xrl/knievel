# Release Playbook

What to do when a tag build fails halfway, when a bad release ships,
or when an operator needs to roll back. Pairs with
`RELEASE_CHECKLIST.md` (the pre-tag gate) and `DEPLOYMENT.md` § 9
(rolling restarts during normal upgrades).

This document gets fleshed out over time as real-world incidents
surface gaps; the sections below are the v0 starting frame.

## When to use which procedure

| Situation | Procedure |
|---|---|
| Tag pushed; image builds OK; gem regen failed | § 1 — partial-failure recovery |
| Tag pushed; image + gem out; later found broken | § 2 — yank + patch release |
| Operator already pulled a bad image and is running it | § 3 — operator-side rollback |
| Tag pushed but never finished — workflow killed mid-run | § 4 — re-run-on-same-tag |

## 1. Partial-failure recovery

The two release workflows (`release.yml` for image+CLI+GitHub
Release, `release-ruby-gem.yml` for the gem) run in parallel on
each `v*` tag. They share no state — one can succeed while the
other fails. Pattern:

1. **Identify which workflow failed** and at which step.
   `gh run list --workflow release.yml --limit 5`,
   `gh run list --workflow release-ruby-gem.yml --limit 5`.
2. **If `release.yml` (image) failed:**
   - Image not pushed → fix the cause (build error, ghcr auth,
     cosign quota), `gh workflow run release.yml --ref vX.Y.Z`.
     The workflow is idempotent on its core artifacts (the
     image push uses `--exists=skip`-equivalent semantics; cosign
     re-signs cleanly).
   - Image pushed but later steps failed (CLI binaries, GitHub
     Release) → re-run; the image-push step's idempotency carries.
3. **If `release-ruby-gem.yml` (gem) failed:**
   - Generation failed → fix the cause, re-trigger. The workflow
     refuses to overwrite an existing tag on `knievel-ruby` (see
     `release-ruby-gem.yml` "Commit + tag knievel-ruby" step), so
     a partial commit on `knievel-ruby/main` doesn't block re-runs;
     a partial tag does.
   - Gem built but `gem push` to RubyGems failed → manually
     `gem install` the workflow artifact and `gem push` from a
     trusted machine; record this in the release PR.
4. **Don't tag a different version** to "try again." Rotate the
   patch version (`vX.Y.(Z+1)`) only if the failed run actually
   published an artifact a real consumer might have pulled.

## 2. Yank + patch release

A bad release got out. RubyGems and ghcr behave differently:

### RubyGems

`gem yank knievel -v X.Y.Z` removes the version from the index.
Bundler / `gem install` no longer pulls it; existing installs are
unaffected. Yanking is reversible (`gem yank --undo`) but treat it
as final — if you need the version back, cut a new patch.

```sh
gem yank knievel -v X.Y.Z
```

Then ship a patch release (`vX.Y.(Z+1)`) with the fix and a
`Security` (or `Fixed`) entry in CHANGELOG that names the yanked
version explicitly.

### Container image

OCI images on ghcr are **immutable by digest** — the bad image
stays addressable forever via its `sha256:…`. The mutable tag
(`vX.Y.Z`) can be re-pointed by pushing a corrected image at the
same tag, but **don't**: it confuses anyone who pulled the old
digest. Instead:

1. Cut a patch tag `vX.Y.(Z+1)` with the fix.
2. Push the corrected image at `vX.Y.(Z+1)`.
3. Re-point the floating `latest` tag to `vX.Y.(Z+1)` (the
   `release.yml` workflow does this automatically on tag).
4. Annotate the bad release on the GitHub Releases page with a
   "DO NOT USE — see `vX.Y.(Z+1)`" notice. Don't delete the bad
   release.

## 3. Operator-side rollback

An operator who pulled a bad image needs to roll back. The Helm
chart and the compose manifest both pin by tag (or digest, for
immutable installs). Procedure:

```sh
# Helm:
helm upgrade knievel oci://ghcr.io/knievel-ads/charts/knievel \
  --version <previous-good> \
  --set image.tag=v<previous-good>
# Or pin a digest for safety:
helm upgrade knievel oci://… \
  --set image.repository=ghcr.io/knievel-ads/knievel \
  --set image.tag=sha256:<previous-good-digest>

# Compose:
KNIEVEL_IMAGE=ghcr.io/knievel-ads/knievel:v<previous-good> \
  docker compose -f examples/compose/compose.yaml up -d
```

The pod rollout takes the standard rolling-restart shape from
`DEPLOYMENT.md` § 9. The snapshot rebuilds from Postgres on each
new pod's boot; the events channel buffer drains within ~2 s
because the flusher's batch interval doesn't change across versions.
**No data loss for already-buffered events** during the rollback.

If the rollback is for a security CVE, also rotate any secrets the
bad version touched (HMAC signing secret, auth-token re-mint if
the bug exposed token contents).

## 4. Re-run-on-same-tag

A workflow killed mid-run (e.g., GitHub Actions runner timeout)
hasn't published anything user-visible yet. The fix is to re-run
the workflow:

```sh
gh run rerun <run-id> --failed
# Or, force a fresh run on the same tag:
gh workflow run release.yml --ref vX.Y.Z
```

Both `release.yml` and `release-ruby-gem.yml` are designed to be
re-run-safe on the same tag — the image push is idempotent by
digest; the gem regen refuses to overwrite an existing
`knievel-ruby` tag (so a clean re-run requires deleting the
partial tag from `knievel-ruby` first if the prior run got that
far).

## 5. Communicating the incident

For any user-visible badness:

1. **Cut a patch release** with the fix per § 1 or § 2.
2. **Annotate the bad release** on the GitHub Releases page
   (don't delete it — historical record matters).
3. **Update CHANGELOG.md** in the next release with a `Security`
   or `Fixed` entry that explicitly names the bad version.
4. **Post-mortem** for any incident that affected a real consumer.
   Living doc — no template yet; add one when the first one
   ships. Reference: GitHub's blameless post-mortem culture.

## 6. Things that intentionally have no playbook entry

- **Force-push to `main`.** Don't. The harness git-safety rules
  reject it; `RELEASE_CHECKLIST.md` requires a proper patch tag
  to fix anything.
- **`git tag --force` to overwrite an existing tag.** Don't. If a
  tag is bad, cut the next patch. Existing immutable artifacts
  (image digests, RubyGems versions) are not in your power to
  retract once observed.
- **Roll forward by editing the docker image in-place.** ghcr
  digests are immutable; this is impossible by design. Cut the
  next patch instead.

## References

- `RELEASE_CHECKLIST.md` — the gate.
- `DEPLOYMENT.md` § 9 — normal upgrade rolling-restart flow.
- `TESTING.md` § 12.9 — the release-tagging workflow contract.
- `REQUIREMENTS.md` § 6.4 — the additive-forever compatibility
  policy that constrains what a patch release can do.
