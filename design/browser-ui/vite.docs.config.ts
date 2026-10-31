import { defineConfig } from 'vite-plus';
import { fileURLToPath } from 'node:url';

// Dev-only design-system docs and mockups (#5821): `vp dev --config vite.docs.config.ts`, on one
// fixed local URL, http://127.0.0.1:5100/. Like the gallery, nothing here reaches a product bundle.
export default defineConfig({
  root: fileURLToPath(new URL('./docs/', import.meta.url)),
  appType: 'spa',
  server: {
    host: '127.0.0.1',
    port: 5100,
    strictPort: true,
    watch: { usePolling: true, interval: 250 },
    fs: { allow: [fileURLToPath(new URL('../', import.meta.url))] },
  },
});
