import { execFileSync } from 'node:child_process';
import { expect, type Locator, type Page } from '@playwright/test';
import type { Door, PairedBrowser } from './browser.js';
import type { AcceptanceWorld } from './world.js';

/** Run a real binary with the world's environment; fails with its output. */
export function run(
  world: AcceptanceWorld,
  binary: string,
  args: string[],
  input?: string,
): string {
  try {
    return execFileSync(binary, args, {
      env: world.env(),
      encoding: 'utf8',
      input,
      stdio: [input === undefined ? 'ignore' : 'pipe', 'pipe', 'pipe'],
    });
  } catch (error) {
    const failed = error as { stdout?: string; stderr?: string };
    throw new Error(`${binary} ${args.join(' ')} failed: ${failed.stdout}${failed.stderr}`, {
      cause: error,
    });
  }
}

/** The paired device's Remote client ID, for `devices revoke`. */
export function clientId(world: AcceptanceWorld, name: string): string {
  const listing = JSON.parse(run(world, world.binaries.remote, ['devices', '--json'])) as {
    devices: { clientId: string; name: string; revoked: boolean }[];
  };
  const device = listing.devices.find((d) => d.name === name && !d.revoked);
  if (!device) throw new Error(`No active paired device named ${name}`);
  return device.clientId;
}

export interface CreatedPage {
  pageId: string;
  /** Relative to the Remote door address: `x/colab/#space=<space>&path=%2Fpages%2F<page>`. */
  path: string;
}

/** Create a private page through the real `tmt colab page create` (source on stdin). */
export function createPage(world: AcceptanceWorld, title: string, html: string): CreatedPage {
  const created = JSON.parse(
    run(
      world,
      world.binaries.colab,
      ['page', 'create', '--title', title, '--file', '-', '--json'],
      html,
    ),
  ) as { pageId: string; path: string };
  return { pageId: created.pageId, path: created.path };
}

/** Open a created page under the Remote door as this paired device. */
export async function openPage(
  door: Pick<Door, 'address'>,
  browser: PairedBrowser,
  created: CreatedPage,
): Promise<Page> {
  const page = await browser.context.newPage();
  await page.goto(`${door.address}/${created.path}`);
  return page;
}

/** Select the text of one element inside the sandboxed renderer frame. */
export async function selectInRenderer(page: Page, selector: string): Promise<void> {
  await page
    .frameLocator('iframe')
    .locator(selector)
    .evaluate((node) => {
      const range = document.createRange();
      range.selectNodeContents(node);
      const selection = node.ownerDocument.getSelection()!;
      selection.removeAllRanges();
      selection.addRange(range);
    });
  await expect(page.getByTestId('ask-action')).toBeEnabled();
}

/** Choose an annotation recipient through the shared parent input listbox. */
export async function annotationInput(container: Locator, agent: string) {
  const input = container.getByRole('combobox', { name: 'Message to agent', exact: true });
  await input.fill('@');
  await container
    .getByRole('option')
    .filter({ hasText: `@${agent} ·` })
    .click();
  return input;
}

export interface PreviewedAsk {
  operationId: string;
  /** Exact text of the preview: the Remote device-name line plus the transport message. */
  previewText: string;
}

/** Drive the real Ask UI up to, but not including, Send. */
export async function previewAsk(
  page: Page,
  agentId: string,
  question: string,
): Promise<PreviewedAsk> {
  if ((await page.getByTestId('ask-toggle').getAttribute('aria-expanded')) === 'false')
    await page.getByTestId('ask-toggle').click();
  await page.getByTestId('ask-action').click();
  await page.getByRole('combobox', { name: /Choose an agent/ }).click();
  await page.locator(`[data-testid=ask-agent-option][data-agent-id="${agentId}"]`).click();
  await page.getByLabel('Question or instruction').fill(question);
  await page.getByRole('button', { name: 'Ask agent — preview', exact: true }).click();
  const preview = page.getByTestId('ask-preview');
  await expect(preview).toBeVisible();
  const operationId = await preview.getAttribute('data-operation-id');
  const previewText = await page.getByTestId('ask-preview-text').textContent();
  if (!operationId || previewText === null) throw new Error('Preview did not freeze an ask');
  return { operationId, previewText };
}

export const send = (page: Page) => page.getByTestId('ask-send').click();

export const askEntry = (page: Page, operationId: string): Locator =>
  page.locator(`[data-testid=ask-entry][data-operation-id="${operationId}"]`);

export const askState = (page: Page, operationId: string): Locator =>
  askEntry(page, operationId).getByTestId('ask-state');

/** A free loopback port, so Remote can restart on the same origin. */
export async function freePort(): Promise<number> {
  const { createServer } = await import('node:net');
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address() as { port: number };
      server.close(() => resolve(port));
    });
  });
}
