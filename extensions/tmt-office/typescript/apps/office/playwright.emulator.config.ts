import { defineConfig } from '@playwright/test';
import common from './playwright.config.js';

/** Emulator-backed contracts exclude local-only browser and native companion ownership. */
export default defineConfig({
  ...common,
  testIgnore: [
    'local-office.spec.ts',
    'local-office-*.spec.ts',
    'modular-robot-scene.spec.ts',
    'native-local-*.spec.ts',
  ],
});
