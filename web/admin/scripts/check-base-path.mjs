// Post-build guard. The SPA is mounted under /admin/ by knievel's
// poem server (mount_admin_ui in src/server.rs), and Vite's
// `base: '/admin/'` config rewrites built asset URLs to match. If
// either side drifts (e.g. someone deletes the base option), the
// rendered HTML emits root-relative `/assets/...` URLs that 404
// behind the mount and the page goes blank — the very bug PR #14
// fixed. This check fails the build before the regression escapes.

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const html = readFileSync(resolve('dist/index.html'), 'utf8');

const bad = html.match(/(?:src|href)="\/assets\//g);
if (bad) {
  console.error(
    'check-base-path: dist/index.html references /assets/ (root-anchored).',
  );
  console.error(
    'Vite `base: "/admin/"` should rewrite these to /admin/assets/.',
  );
  process.exit(1);
}

const good = html.match(/(?:src|href)="\/admin\/assets\//g);
if (!good) {
  console.error(
    'check-base-path: dist/index.html has no /admin/assets/ references.',
  );
  console.error('Did the build emit any assets at all?');
  process.exit(1);
}

console.log(`check-base-path: ok (${good.length} /admin/assets/ refs)`);
