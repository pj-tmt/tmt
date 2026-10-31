import { readFileSync } from 'node:fs';
import { expect, test } from 'vite-plus/test';
import * as settings from '../src/colab-settings-source.js';

const native = (name: string) =>
  readFileSync(new URL(`../../../rust/tmt-colab/src/${name}.rs`, import.meta.url), 'utf8');

test('reference defaults agree with the cited native definitions', () => {
  // settings.rs::ColabSettings::open owns the absent-key default.
  expect(native('settings')).toContain('self.open.unwrap_or(true)');
  // page_expiry.rs::DEFAULT_DAYS owns this default.
  const expiry = Number(native('page_expiry').match(/pub const DEFAULT_DAYS: u64 = (\d+);/)?.[1]);
  expect(Number.isSafeInteger(expiry)).toBe(true);
  expect(settings).toHaveProperty('colabSettingsReferenceDefaults', {
    open: true,
    pageExpiryDays: expiry,
    sample: true,
  });
});
