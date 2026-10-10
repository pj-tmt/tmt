import { expect, type Page } from '@playwright/test';
import { pageAction } from './page-actions.js';
import { text } from '../src/strings.js';
import { annotationInput } from '../acceptance/harness/ask.js';

export const fixture = '/test/ask-page-browser.tsx';
// A 1x1 PNG: a real raster the parent may preview.
export const PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==',
  'base64',
);

export async function mount(page: Page) {
  await page.goto('/');
  await page.evaluate(async ({ fixture }) => (await import(fixture)).mount({ attachments: true }), {
    fixture,
  });
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
}
export const actions = async (page: Page): Promise<string[]> =>
  page.evaluate(async (path) => (await import(path)).proof().actions, fixture);
export const threads = async (page: Page) =>
  page.evaluate(async (path) => (await import(path)).discussionProof(), fixture);
export async function openChat(page: Page) {
  const host = page.locator('#ask-page-fixture');
  const toggle = await pageAction(host, 'Chat');
  await toggle.click();
  const panel = page.getByTestId('chat-panel');
  return { panel, input: await annotationInput(panel, 'Agent 1') };
}
export async function attach(
  page: Page,
  files: { name: string; mimeType: string; buffer: Buffer }[],
) {
  const chooser = page.waitForEvent('filechooser');
  await page.getByRole('button', { name: text.attachFiles }).click();
  await (await chooser).setFiles(files);
}
export const send = (page: Page) => page.getByRole('button', { name: text.askSend, exact: true });
