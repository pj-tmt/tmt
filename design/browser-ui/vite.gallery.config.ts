import { defineConfig } from 'vite-plus';
import { fileURLToPath } from 'node:url';

// Dev-only design gallery: `vp dev --config vite.gallery.config.ts`. The leaf has no build and
// exports nothing from `gallery/`, so the gallery never reaches a product bundle.
export default defineConfig({
  root: fileURLToPath(new URL('./gallery/', import.meta.url)),
  server: {
    host: '127.0.0.1',
    port: 5183,
    strictPort: true,
    // Sandboxed shells get no FSEvents; polling keeps HMR working there.
    watch: { usePolling: true, interval: 250 },
    // The gallery reads the leaf's sources and the shared tokens one level up.
    fs: { allow: [fileURLToPath(new URL('../', import.meta.url))] },
  },
});
