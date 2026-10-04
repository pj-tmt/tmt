import { execFileSync } from 'node:child_process';
import { expect, type Locator, type Page } from '@playwright/test';
import type { Door, PairedBrowser } from './browser.js';
import type { AcceptanceWorld, Agent } from './world.js';

/** Run a real binary with the world's environment; fails with its output. */
export function run(
  world: AcceptanceWorld,
  binary: string,
  args: string[],
  input?: string,
  callerPane?: string,
): string {
  try {
    return execFileSync(binary, args, {
      env: world.env(callerPane),
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
export function createPage(
  world: AcceptanceWorld,
  title: string,
  html: string,
  callerPane?: string,
): CreatedPage {
  const created = JSON.parse(
    run(
      world,
      world.binaries.colab,
      ['page', 'create', '--title', title, '--file', '-', '--json'],
      html,
      callerPane,
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
  await expect(page.getByTestId('selection-ask')).toBeVisible();
}

/** Choose an annotation recipient through the shared parent input listbox. */
export async function annotationInput(container: Locator, agent: string) {
  const input = container.getByRole('combobox', { name: 'Message to agent', exact: true });
  await input.fill('');
  await input.fill('@');
  await container
    .page()
    .getByRole('option')
    .filter({ hasText: `@${agent} ·` })
    .click();
  return input;
}

export interface ComposedChat {
  /** The text the recipient received for this turn; it exists only after Enter has sent it. */
  delivered(): string;
}
export async function openChat(page: Page) {
  if (await page.locator('.page-drawer[data-panel=chat][open]').isVisible()) return;
  const toggle = page.getByTestId('chat-toggle');
  await expect(toggle).toBeAttached();
  if (!(await toggle.isVisible()))
    await page.getByRole('button', { name: 'More page actions' }).click();
  await toggle.click();
  await expect(page.getByTestId('chat-panel')).toBeVisible();
}
/** Compose one plain input, with no effect before Enter. */
export async function composeChat(
  page: Page,
  agent: Agent,
  question: string,
): Promise<ComposedChat> {
  await openChat(page);
  const panel = page.getByTestId('chat-panel');
  const input = await annotationInput(panel, agent.name);
  await input.fill(`@${agent.name} ${question}`);
  const turn = agent.received().length;
  return {
    delivered() {
      const row = agent.received()[turn];
      if (typeof row?.message !== 'string') throw new Error('The turn was not received yet');
      return row.message;
    },
  };
}
/** Enter creates the frozen operation; discover its ID only from the admitted stream. */
export async function sendChat(page: Page, composed: ComposedChat) {
  const panel = page.getByTestId('chat-panel');
  const earlier = await panel
    .getByTestId('ask-entry')
    .evaluateAll((nodes) => nodes.map((node) => (node as HTMLElement).dataset.operationId));
  await panel.getByRole('combobox', { name: 'Message to agent' }).press('Enter');
  const entries = panel.getByTestId('ask-entry');
  await expect
    .poll(
      async () =>
        (
          await entries.evaluateAll((nodes) =>
            nodes.map((node) => (node as HTMLElement).dataset.operationId),
          )
        ).filter((id) => !earlier.includes(id)).length,
    )
    .toBe(1);
  const operationId = (
    await entries.evaluateAll((nodes) =>
      nodes.map((node) => (node as HTMLElement).dataset.operationId),
    )
  ).find((id) => !earlier.includes(id));
  if (!operationId) throw new Error('No admitted Chat operation');
  return { ...composed, operationId };
}

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
