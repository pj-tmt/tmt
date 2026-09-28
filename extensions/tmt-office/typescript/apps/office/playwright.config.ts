import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  testIgnore: ['native-local-prop-capacity.spec.ts', 'native-local-world-capacity.spec.ts'],
  workers: 1,
  retries: 0,
  timeout: 30_000,
  expect: { timeout: 10_000 },
  reporter: 'list',
  use: { browserName: 'chromium', baseURL: 'http://127.0.0.1:4173' },
  webServer: [
    {
      command:
        'pnpm exec vite preview --host 127.0.0.1 --port 4175 --strictPort --outDir dist-cloud',
      url: 'http://127.0.0.1:4175',
      reuseExistingServer: false,
    },
    {
      command: 'pnpm exec vite preview --host 127.0.0.1 --port 4173 --strictPort',
      url: 'http://127.0.0.1:4173',
      reuseExistingServer: false,
    },
    {
      command:
        'pnpm exec vite preview --host 127.0.0.1 --port 4174 --strictPort --outDir dist-preview',
      url: 'http://127.0.0.1:4174',
      reuseExistingServer: false,
    },
    {
      command: 'pnpm exec vite --mode offline --host 127.0.0.1 --port 4176 --strictPort',
      url: 'http://127.0.0.1:4176/local',
      reuseExistingServer: false,
    },
  ],
});
