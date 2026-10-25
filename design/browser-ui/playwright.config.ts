import { defineConfig } from '@playwright/test';
const port = Number(process.env.BROWSER_UI_TEST_PORT ?? 5184);
if (!Number.isSafeInteger(port) || port < 1 || port > 65535)
  throw new Error('BROWSER_UI_TEST_PORT must be a TCP port.');
const baseURL = `http://127.0.0.1:${port}`;
// Real-browser checks of the design gallery's own pages: computed styles and overlay motion. The
// gallery is served by the leaf's dev server; nothing here touches a product or the host tmux.
export default defineConfig({
  testDir: './e2e',
  workers: 1,
  retries: 0,
  timeout: 60000,
  use: { baseURL, trace: 'retain-on-failure' },
  webServer: {
    command: `./node_modules/.bin/vp dev --config vite.gallery.config.ts --port ${port} --strictPort`,
    url: baseURL,
    reuseExistingServer: false,
  },
  projects: [{ name: 'chromium', use: { browserName: 'chromium' } }],
});
