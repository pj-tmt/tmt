import { defineConfig } from '@playwright/test';
import common from './playwright.config.js';

/** Inventory-only union of required acceptance and explicit capacity diagnostics. */
export default defineConfig({
  ...common,
  testIgnore: [],
});
