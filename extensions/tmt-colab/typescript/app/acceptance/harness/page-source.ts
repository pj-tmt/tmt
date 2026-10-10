import { pageAction } from '../../test/page-actions.js';
import { expect, type Page } from '@playwright/test';
import { createHash } from 'node:crypto';
import { text } from '../../src/strings.js';

/** Incompressible page source of exactly `bytes` characters, so a size here is the size on the wire. */
export const distinct = (bytes: number, seed: string) => {
  let out = '';
  for (let i = 0; out.length < bytes; i++)
    out += createHash('sha256').update(`${seed}:${i}`).digest('hex');
  return `<p>${out.slice(0, bytes - 7)}</p>`;
};

export const source = (page: Page) => page.getByRole('textbox', { name: 'Source', exact: true });
export async function openSource(page: Page) {
  await (await pageAction(page, 'Source')).click({ timeout: 60_000 });
  return source(page);
}
// A save is done when the editor leaves "Saving…": the reply arrives after the serve has combined
// the new tail, so a CLI write that follows sees room for its own update.
export async function save(page: Page) {
  await page.getByRole('button', { name: text.save, exact: true }).click();
  await expect(page.getByRole('button', { name: text.saving, exact: true })).toHaveCount(0, {
    timeout: 120_000,
  });
}
