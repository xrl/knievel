// `defineConfig` from vitest/config widens Vite's options with the
// `test:` block, so vite.config.ts and vitest config can live in
// one file with proper typing.
import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import { TanStackRouterVite } from '@tanstack/router-plugin/vite';

export default defineConfig({
  plugins: [
    // Router plugin runs before @vitejs/plugin-react so the
    // generated routeTree.gen.ts is in place when react picks
    // up the source tree.
    TanStackRouterVite({ target: 'react', autoCodeSplitting: true }),
    react(),
  ],
  server: {
    port: 5173,
    strictPort: true,
  },
  test: {
    globals: true,
    environment: 'happy-dom',
    setupFiles: ['./src/test/setup.ts'],
    css: true,
    // TanStack Router file-based routes use dot-segments
    // (`reports.test.tsx` → `/reports/test`), which collide
    // with vitest's default `*.test.tsx` glob. Constrain
    // the test discovery to files explicitly under `test/`
    // OR matching `*.spec.{ts,tsx}` so route filenames
    // can't accidentally be picked up as test suites.
    include: ['src/**/*.{spec,test}.{ts,tsx}'],
    exclude: ['**/node_modules/**', '**/dist/**', 'src/routes/**'],
  },
});
