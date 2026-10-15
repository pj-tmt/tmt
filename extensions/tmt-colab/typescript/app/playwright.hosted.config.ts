import { defineConfig } from '@playwright/test';

/** Explicit emulator harness only; ordinary browser/acceptance runs never start providers. */
export default defineConfig({
  testDir: './acceptance',
  testMatch: 'hosted-reader.proof.ts',
  workers: 1,
  retries: 0,
  timeout: 180000,
  reporter: 'list',
  use: { trace: 'retain-on-failure' },
});
