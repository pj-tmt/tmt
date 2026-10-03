import { defineConfig } from '@playwright/test';
// Real-binary Ask agent acceptance (#1110). It spawns the built tmt, tmt-remote
// and tmt-colab itself, so unlike playwright.config.ts it starts no dev server.
export default defineConfig({
  testDir: './l5',
  testMatch: '**/*.spec.ts',
  workers: 1,
  retries: 0,
  timeout: 120000,
  reporter: 'list',
  use: { trace: 'retain-on-failure' },
});
