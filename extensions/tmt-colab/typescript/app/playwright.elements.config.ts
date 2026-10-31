import { defineConfig } from '@playwright/test';
import { fileURLToPath } from 'node:url';
// One headless verification of the shared adapters and their real Colab consumers.
const leaf = fileURLToPath(new URL('../../../../design/browser-ui/', import.meta.url));
export default defineConfig({
  testDir: '../../../..',
  workers: 1,
  retries: 0,
  timeout: 60000,
  use: { browserName: 'chromium', reducedMotion: 'reduce', trace: 'retain-on-failure' },
  webServer: [
    {
      cwd: leaf,
      command:
        './node_modules/.bin/vp dev --config vite.gallery.config.ts --port 15383 --strictPort',
      url: 'http://127.0.0.1:15383',
      reuseExistingServer: false,
    },
    {
      command: './node_modules/.bin/vp dev --host 127.0.0.1 --port 15379 --strictPort',
      url: 'http://127.0.0.1:15379',
      reuseExistingServer: false,
    },
  ],
  projects: [
    {
      name: 'elements',
      testMatch: '**/design/browser-ui/e2e/elements.spec.ts',
      use: { baseURL: 'http://127.0.0.1:15383' },
    },
    {
      name: 'catalog',
      testMatch: '**/app/e2e/design-catalog.spec.ts',
      use: { baseURL: 'http://127.0.0.1:15379' },
    },
  ],
});
