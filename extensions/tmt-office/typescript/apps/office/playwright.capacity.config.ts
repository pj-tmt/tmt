import { defineConfig } from '@playwright/test';
import common from './playwright.config.js';

/** Explicit capacity diagnostics stay outside required browser acceptance. */
export default defineConfig({
  ...common,
  testIgnore: [],
  testMatch: ['native-local-prop-capacity.spec.ts', 'native-local-world-capacity.spec.ts'],
  webServer: undefined,
  timeout: 120_000,
  use: {
    ...common.use,
    channel: process.env.TMT_TEST_BROWSER_CHANNEL ?? 'chrome',
  },
});
