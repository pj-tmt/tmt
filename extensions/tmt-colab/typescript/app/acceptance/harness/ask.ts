import { pageAction } from '../../test/page-actions.js';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
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
      maxBuffer: 64 * 1024 * 1024,
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
  // Creation captures optional provenance through the real public Remote status command.
  world.linkExtensions();
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
  // Paired viewers have separate browser contexts; keyboard checks target this viewer.
  await page.bringToFront();
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

/** Find the shared field; the caller supplies the visible recipient mention in its draft. */
export async function annotationInput(container: Locator, _agent: string) {
  const input = container.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toBeVisible();
  // Directory readiness is status-only. No hidden recipient selection is made here.
  await expect(container.locator('.annotation-status-row')).not.toContainText('Checking');
  return input;
}

/**
 * The message text a composer field holds: one paragraph per line joined by LF, a mention as its
 * token. The mention chip's dot, removal control and machine suffix are presentation, and
 * `innerText` adds a line break around the chip's inline-flex box, so it is not the sent text.
 */
export async function composerText(field: Locator): Promise<string> {
  return field.evaluate((root) => {
    const read = (node: Node): string => {
      if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? '';
      if (!(node instanceof HTMLElement)) return '';
      if (node.classList.contains('message-mention')) {
        return node.querySelector('.message-mention-name')?.textContent ?? '';
      }
      return Array.from(node.childNodes, read).join('');
    };
    return Array.from(root.children, read).join('\n');
  });
}

export interface ComposedChat {
  /** The text the recipient received for this turn; it exists only after Enter has sent it. */
  delivered(): string;
}
export async function openChat(page: Page) {
  if (await page.locator('.page-drawer[data-panel=chat][open]').isVisible()) return;
  const toggle = await pageAction(page, 'Chat');
  await expect(toggle).toBeAttached();
  await toggle.click();
  await expect(page.getByTestId('chat-panel')).toBeVisible();
}
/** Compose the visible mention and question, with no effect before Enter. */
export async function composeChat(
  page: Page,
  agent: Agent,
  question: string,
): Promise<ComposedChat> {
  await openChat(page);
  const panel = page.getByTestId('chat-panel');
  const input = panel.getByRole('combobox', { name: 'Message', exact: true });
  const draft = `@${agent.name} ${question}`;
  await input.fill(draft);
  await expect.poll(() => composerText(input)).toBe(draft);
  await annotationInput(panel, agent.name);
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
  await panel.getByRole('combobox', { name: 'Message' }).press('Enter');
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

/** Opt-in evidence only: ordinary acceptance remains independent of specialized manifests. */
function composerEvidence(label: string) {
  const directory = process.env.COLAB_1817_NATIVE_TRACE_DIR;
  const file = process.env.COLAB_COMPOSER_ASSET_MANIFEST;
  const head = process.env.COLAB_COMPOSER_EXPECTED_HEAD;
  if ([directory, file, head].every((value) => value === undefined)) return undefined;
  if (
    !directory ||
    !path.isAbsolute(directory) ||
    !file ||
    !path.isAbsolute(file) ||
    !head ||
    !/^[a-f0-9]{40}$/.test(head) ||
    !/^[a-z-]+$/.test(label)
  )
    throw new Error('Set complete valid frozen composer evidence configuration.');
  const manifest = JSON.parse(fs.readFileSync(file, 'utf8')) as {
    head: string;
    policies: { parent: string; renderer: string };
    distAssets: { path: string; sha256: string; size: number }[];
  };
  expect(manifest.head).toBe(head);
  expect(manifest.distAssets).toHaveLength(11);
  expect(new Set(manifest.distAssets.map((asset) => asset.path)).size).toBe(11);
  for (const asset of manifest.distAssets) {
    if (
      !/^(?:assets\/[A-Za-z0-9_.-]+|[A-Za-z0-9_.-]+)$/.test(asset.path) ||
      !/^[a-f0-9]{64}$/.test(asset.sha256) ||
      !Number.isSafeInteger(asset.size) ||
      asset.size <= 0
    )
      throw new Error('Invalid frozen asset entry.');
  }
  if (
    typeof manifest.policies.parent !== 'string' ||
    !manifest.policies.parent ||
    typeof manifest.policies.renderer !== 'string' ||
    !manifest.policies.renderer ||
    /unsafe-inline|unsafe-eval/.test(manifest.policies.parent)
  )
    throw new Error('Invalid frozen composer CSP.');
  return { directory, head, manifest };
}

/** Original context trace and root witness; lifecycle cleanup stays with AcceptanceWorld. */
export async function composerTrace(
  world: AcceptanceWorld,
  browser: PairedBrowser,
  origin: string,
  label: string,
) {
  const evidence = composerEvidence(label);
  if (!evidence) return;
  const { directory, head } = evidence;
  fs.mkdirSync(directory, { recursive: true });
  const trace = path.join(directory, `${label}.zip`);
  const witness = path.join(directory, `${label}.world.json`);
  if (fs.existsSync(trace) || fs.existsSync(witness))
    throw new Error('Refuse to overwrite original evidence.');
  fs.writeFileSync(
    witness,
    JSON.stringify(
      {
        label,
        workerPid: process.pid,
        root: world.root,
        profile: browser.profile,
        tmuxName: world.tmuxName,
        sockets: world.sockets(),
        origin: new URL(origin).origin,
        expectedHead: head,
      },
      null,
      2,
    ),
    { flag: 'wx' },
  );
  await browser.context.tracing.start({ screenshots: true, snapshots: true, sources: true });
  world.onDispose(() => browser.context.tracing.stop({ path: trace }));
}

/** Read every frozen embedded asset so original trace resources carry all eleven bodies. */
export async function composerAssets(page: Page, label: string) {
  const evidence = composerEvidence(label);
  if (!evidence) return;
  const { directory, head, manifest } = evidence;
  const mount = new URL('./', page.url());
  const observed: { path: string; size: number; sha256: string }[] = [];
  for (const asset of manifest.distAssets) {
    if (!/^(?:assets\/[A-Za-z0-9_.-]+|[A-Za-z0-9_.-]+)$/.test(asset.path))
      throw new Error('Invalid frozen asset path.');
    const response = await page.context().request.get(new URL(asset.path, mount).href);
    expect(response.status(), asset.path).toBe(200);
    const body = await response.body();
    expect(body.length, asset.path).toBe(asset.size);
    const sha256 = createHash('sha256').update(body).digest('hex');
    expect(sha256, asset.path).toBe(asset.sha256);
    observed.push({ path: asset.path, size: body.length, sha256 });
    expect(response.headers()['content-security-policy'], asset.path).toBe(
      asset.path === 'renderer.html' ? manifest.policies.renderer : manifest.policies.parent,
    );
  }
  fs.writeFileSync(
    path.join(directory, `${label}.assets.json`),
    JSON.stringify({ head, origin: mount.origin, mount: mount.pathname, observed }, null, 2),
    { flag: 'wx' },
  );
}
