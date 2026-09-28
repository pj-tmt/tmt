import { defineConfig } from '@playwright/test';
import common from './playwright.config.js';

/** Native fixtures own their temporary services; no Vite servers or cloud setup. */
export default defineConfig({
  ...common,
  webServer: undefined,
  testMatch: 'native-local-*.spec.ts',
  timeout: 120_000,
  use: {
    ...common.use,
    channel: process.env.TMT_TEST_BROWSER_CHANNEL ?? 'chrome',
  },
});
