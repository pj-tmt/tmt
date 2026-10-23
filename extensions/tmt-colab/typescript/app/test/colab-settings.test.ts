import { readFileSync } from 'node:fs';
import { expect, test } from 'vite-plus/test';
import * as settings from '../src/colab-settings-source.js';

const native = (name: string) =>
  readFileSync(new URL(`../../../rust/tmt-colab/src/${name}.rs`, import.meta.url), 'utf8');

test('reference defaults agree with the cited native definitions', () => {
  // settings.rs::ColabSettings::open owns the absent-key default.
  expect(native('settings')).toContain('self.open.unwrap_or(true)');
  // ColabSettings derives Default, so its optional repository starts absent.
  expect(native('settings')).toMatch(/derive\([^\n]*Default[^\n]*\)/);
  expect(native('settings')).toContain('github_repo: Option<Repo>');
  // wip.rs::DEFAULT_LIMIT and page_expiry.rs::DEFAULT_DAYS own these defaults.
  const wip = Number(native('wip').match(/pub const DEFAULT_LIMIT: u32 = (\d+);/)?.[1]);
  const expiry = Number(native('page_expiry').match(/pub const DEFAULT_DAYS: u64 = (\d+);/)?.[1]);
  expect(Number.isSafeInteger(wip)).toBe(true);
  expect(Number.isSafeInteger(expiry)).toBe(true);
  expect(settings).toHaveProperty('colabSettingsReferenceDefaults', {
    open: true,
    wipLimit: { default: wip, agents: {} },
    pageExpiryDays: expiry,
    sample: true,
  });
});
