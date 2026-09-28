import { defineConfig } from '@playwright/test';
import common from './playwright.config.js';

/** Focused, opt-in pixel review. Native persistence remains covered by native scenarios. */
export default defineConfig({
  ...common,
  testMatch: 'visual/*.visual.ts',
  testIgnore: [],
  updateSnapshots: 'none',
  use: {
    ...common.use,
    viewport: { width: 1440, height: 1000 },
    deviceScaleFactor: 1,
    reducedMotion: 'reduce',
    colorScheme: 'dark',
    locale: 'en-US',
    timezoneId: 'UTC',
  },
  expect: {
    ...common.expect,
    toHaveScreenshot: { animations: 'disabled', caret: 'hide', maxDiffPixels: 0 },
  },
  webServer: {
    command: 'pnpm exec vite --mode offline --host 127.0.0.1 --port 4176 --strictPort',
    url: 'http://127.0.0.1:4176/local',
    reuseExistingServer: false,
  },
});
