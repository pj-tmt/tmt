import { defineConfig } from '@playwright/test';
/** Explicit qualifying lane; ordinary browser smoke never silently skips or starts an emulator. */
export default defineConfig({
  testDir: '.',
  testMatch: 'browser.spec.ts',
  workers: 1,
  retries: 0,
  timeout: 60000,
  reporter: 'list',
  use: { trace: 'off', headless: true },
});
