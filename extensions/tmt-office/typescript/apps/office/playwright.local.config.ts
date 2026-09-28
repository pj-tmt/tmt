import { defineConfig } from '@playwright/test';
import common from './playwright.config.js';

/** Local browser composition uses the built Vite variants without Firebase emulators. */
export default defineConfig({
  ...common,
  testMatch: ['local-office.spec.ts', 'local-office-*.spec.ts', 'modular-robot-scene.spec.ts'],
});
