import { defineConfig } from '@playwright/test';
const port = Number(process.env.COLAB_APP_TEST_PORT ?? 4179);
if (!Number.isSafeInteger(port) || port < 1 || port > 65535)
  throw new Error('COLAB_APP_TEST_PORT must be a TCP port.');
const baseURL = `http://127.0.0.1:${port}`;
export default defineConfig({
  testDir: './e2e',
  workers: 1,
  retries: 0,
  timeout: 30000,
  use: { baseURL, trace: 'retain-on-failure' },
  webServer: {
    command: `pnpm dev --port ${port} --strictPort`,
    url: baseURL,
    reuseExistingServer: false,
  },
  projects: [{ name: 'chromium', use: { browserName: 'chromium' } }],
});
