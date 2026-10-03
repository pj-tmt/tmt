import { chromium, expect, type BrowserContext, type Page } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import type { OwnedProcess } from './process.js';
import type { AcceptanceWorld } from './world.js';

export interface Door {
  /** `http://127.0.0.1:<port>/r/<prefix>`, the machine's route prefix. */
  address: string;
  origin: string;
  /** Mounted extension pages live under this path. */
  mounts: string;
  remote: OwnedProcess;
  colab: OwnedProcess;
}

/** Real tmt-remote door with real tmt-colab mounted on its owner-only socket. */
export async function startDoor(world: AcceptanceWorld): Promise<Door> {
  const remote = world.spawn(`remote-serve-${Date.now()}`, world.binaries.remote, [
    'serve',
    '--json',
  ]);
  const descriptor = await remote.event((value) => typeof value.address === 'string');
  const colab = world.spawn(`colab-serve-${Date.now()}`, world.binaries.colab, ['serve', '--json']);
  const ready = await colab.event((value) => value.state === 'mounted');
  if (ready.socket !== path.join(world.dataRoot, 'colab', 'door.sock'))
    throw new Error('Colab mounted outside the isolated data root');
  const address = descriptor.address as string;
  return {
    address,
    origin: new URL(address).origin,
    mounts: `${address}/x/`,
    remote,
    colab,
  };
}

export interface PairedBrowser {
  name: string;
  context: BrowserContext;
  page: Page;
  profile: string;
}

/**
 * Pair a fresh Chromium profile as a device through the real door: the owner
 * side runs `pair --json`, the browser opens the link, both sides show the same
 * four words and the owner confirms. Each device owns its own profile, so two
 * viewers have separate keys, IndexedDB and door cookies.
 */
export async function pairBrowser(world: AcceptanceWorld, name: string): Promise<PairedBrowser> {
  const profile = path.join(world.root, `profile-${name}`);
  fs.mkdirSync(profile, { mode: 0o700 });
  const pair = world.spawn(`pair-${name}`, world.binaries.remote, ['pair', '--json']);
  const offer = await pair.event((value) => typeof value.link === 'string');
  const context = await chromium.launchPersistentContext(profile, { headless: true });
  world.onDispose(() => context.close());
  const page = await context.newPage();
  await page.goto(offer.link as string);
  await page.waitForFunction(() => location.hash === '');
  await page.fill('#name', name);
  await page.click('button');
  await expect(page.locator('#words')).toBeVisible();
  const candidate = await pair.event((value) => value.event === 'candidate');
  await expect(page.locator('#words')).toHaveText(
    `Words: ${(candidate.words as string[]).join(' ')}`,
  );
  pair.child.stdin.write('confirm\n');
  await pair.event((value) => value.reason === 'paired');
  await expect(page.locator('#status')).toHaveText(
    'This browser is paired. You can close this page.',
  );
  await pair.exited;
  return { name, context, page, profile };
}

/** Open the mounted Colab app as this paired device. */
export async function openColab(door: Door, browser: PairedBrowser): Promise<Page> {
  const page = await browser.context.newPage();
  const response = await page.goto(`${door.mounts}colab/`);
  expect(response?.status()).toBe(200);
  return page;
}
