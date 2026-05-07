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
  },
});
