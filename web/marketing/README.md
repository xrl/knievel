# knievel marketing site

Scaffold for the **`knievel-ads/knievel-ads.github.io`** repository
(GitHub org-root site). Lives in this repo so the content evolves
alongside the platform; copy into the org-site repo to publish.

## Layout

```
web/marketing/
├── index.html        landing page (no build, single file)
├── api.html          Redoc rendering of openapi.yaml
├── style.css         minimal CSS — no framework
└── README.md         this file
```

## Bootstrap

```sh
# In a fresh checkout of knievel-ads/knievel-ads.github.io:
cp /path/to/knievel/web/marketing/{index.html,api.html,style.css} .
git add index.html api.html style.css
git commit -m "Initial landing page + Redoc API browser"
git push origin main
```

Then in **Settings → Pages** on the new repo:

- **Source:** *Deploy from a branch*
- **Branch:** `main` / `(root)`

GitHub publishes within a minute. The site lives at
**`https://knievel-ads.github.io`** (org-root URL — no `/knievel/`
suffix because the repo name matches the org).

## How the API browser stays current

`api.html` loads `openapi.yaml` at runtime from the canonical
location:

```
https://raw.githubusercontent.com/knievel-ads/knievel/main/openapi.yaml
```

That means every push to `main` is immediately reflected — no
sync workflow needed. Trade-off: a consumer browsing the API page
sees the *unreleased* spec, not the spec at the latest tag. If
that becomes a problem (e.g. when we have non-trivial deprecations
mid-release-cycle), switch the `spec-url` to a tag-pinned URL and
add a deploy hook to bump it.

## Updating the site

The site content tracks the platform deliberately — when the
README, ARCHITECTURE, or DEPLOYMENT docs change in a way that
affects the elevator-pitch sections (Why / What's in v0 /
Quickstart), update `index.html` here in the same PR. The pattern
matches `web/admin/` (Phase 7 admin UI scaffold): site code lives
in this repo, deployment artifacts live elsewhere.

A future improvement: a CI job that fails when `index.html` falls
behind the README's structured sections beyond a freshness window.
Not in scope for v0.

## Tech choices

- **No build step** — single `index.html` + `api.html` +
  `style.css`. GitHub Pages serves them as-is. Anyone can read,
  edit, and preview locally with `python3 -m http.server`.
- **No framework** — stays under 10 KB of CSS, no JS framework,
  no fonts beyond system stack. Loads on a 3G connection.
- **Redoc CDN** — single `<script>` tag from `cdn.redoc.ly`.
  `api.html` is the one page where we accept a third-party
  runtime dependency; if the CDN goes down, the API browser
  breaks but the rest of the site is fine.

## Local preview

```sh
cd web/marketing
python3 -m http.server 4000
# open http://localhost:4000
```

`api.html` works offline only if `openapi.yaml` is reachable —
the `spec-url` points at GitHub raw, so it needs internet.

## Why org-root and not project pages

Org-root (`knievel-ads.github.io`) gives us a clean URL and room
for future products under the same org. Project-pages
(`knievel-ads.github.io/knievel/`) would have worked but locks
the URL into the repo name forever and confuses future consumers
when knievel is one of several artifacts the org publishes.
