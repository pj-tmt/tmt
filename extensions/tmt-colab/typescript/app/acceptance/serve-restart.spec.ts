import { text } from '../src/strings.js';
import { expect, test, type Page } from '@playwright/test';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { READMIT_DELAYS_MS } from '../src/readmit.js';
import { pairBrowser, startRemoteOnly } from './harness/browser.js';
import {
  clientId,
  composeChat,
  createPage,
  openChat,
  openPage,
  run,
  sendChat,
} from './harness/ask.js';
import { openSource, save } from './harness/page-source.js';
import { until } from './harness/process.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';
import type { AcceptanceWorld } from './harness/world.js';

test.afterEach(disposeActiveWorlds);

// #2929: the serve restarts onto a newer installed release (upgrade-safe serve, #2865) while a
// paired owner has a page open. Remote keeps running, so while the replacement is down its mount
// answers with a bare status; that is the extension restarting, not a verdict on the device.
// The replacement is held down until the page has waited longer than the finite readmission,
// which is what ended the page before.
const BEYOND_READMISSION_MS = READMIT_DELAYS_MS.reduce((total, delay) => total + delay, 0) + 10_000;

interface Upgrade {
  address: string;
  /** Install the newer release; its start is held until `release()`. */
  install(): void;
  release(): void;
  /** The replacement answers `mounted`. */
  replaced(): Promise<void>;
}

async function attachedUpgrade(world: AcceptanceWorld): Promise<Upgrade> {
  world.linkExtensions();
  await startRemoteOnly(world);
  const lib = path.join(world.root, 'lib/tmt-colab');
  const old = path.join(lib, 'releases/old');
  const next = path.join(lib, 'releases/next');
  fs.mkdirSync(old, { recursive: true });
  fs.mkdirSync(next);
  fs.copyFileSync(world.binaries.colab, path.join(old, 'tmt-colab'));
  fs.chmodSync(path.join(old, 'tmt-colab'), 0o755);
  fs.writeFileSync(path.join(old, 'receipt.json'), JSON.stringify({ version: 'running-fixture' }));
  fs.symlinkSync('releases/old', path.join(lib, 'current'));
  const colab = world.spawn('installed-colab', path.join(old, 'tmt-colab'), [
    'serve',
    '--json',
    '--no-open',
  ]);
  const first = await colab.event((value) => value.state === 'mounted');
  expect(first.door).toBe('attached');
  world.recordColabServe();
  const gate = path.join(world.root, 'replacement-may-start');
  const mounted = () =>
    colab.lines.filter((line) => {
      try {
        return (JSON.parse(line) as Record<string, unknown>).state === 'mounted';
      } catch {
        return false;
      }
    }).length;
  return {
    address: String(first.url).replace(/\/x\/colab\/$/, ''),
    install() {
      const prepared = JSON.parse(
        run(world, world.binaries.colab, ['serve', '--upgrade-check', '--json', '--no-open']),
      ) as Record<string, unknown>;
      prepared.version = '9.9.9-upgrade-proof';
      const word = (value: string) => "'" + value.replaceAll("'", "'\"'\"'") + "'";
      const program = `#!/bin/sh
if [ "$2" = '--upgrade-check' ]; then printf '%s\\n' ${word(JSON.stringify(prepared))}; exit 0; fi
while [ ! -e ${word(gate)} ]; do sleep 0.2; done
exec ${word(world.binaries.colab)} "$@"
`;
      fs.writeFileSync(path.join(next, 'tmt-colab'), program, { mode: 0o755 });
      fs.writeFileSync(
        path.join(next, 'receipt.json'),
        JSON.stringify({
          version: prepared.version,
          file_sha256: { 'tmt-colab': createHash('sha256').update(program).digest('hex') },
        }),
      );
      fs.unlinkSync(path.join(lib, 'current'));
      fs.symlinkSync('releases/next', path.join(lib, 'current'));
    },
    release() {
      fs.writeFileSync(gate, '');
    },
    replaced: () => until(() => mounted() >= 2, 'the replacement Colab readiness', 120_000),
  };
}

const status = (page: Page) => page.locator('.status');

test('an open owner page waits out a serve upgrade that outlasts the readmission, then reads, saves and sends once', async () => {
  test.setTimeout(300_000);
  await withWorld(async (world) => {
    const upgrade = await attachedUpgrade(world);
    const agent = await world.startAgent('restart-agent');
    const device = await pairBrowser(world, 'restart-owner', { talk: true });
    const created = createPage(world, 'Serve upgrade', '<h1 id="h">Before</h1>', agent.pane);
    const page = await openPage({ address: upgrade.address }, device, created);
    await expect(page.locator('.status.live .status-label')).toHaveText('Live');
    await openChat(page);
    const navigations: string[] = [];
    page.on('framenavigated', (frame) => {
      if (frame === page.mainFrame()) navigations.push(frame.url());
    });

    upgrade.install();
    // The replacement stays down: the page says so quietly and keeps waiting past the old limit.
    await expect(status(page)).toContainText('Reconnecting…', { timeout: 60_000 });
    await page.waitForTimeout(BEYOND_READMISSION_MS);
    await expect(status(page)).toContainText('Reconnecting…');
    await expect(page.getByRole('heading', { name: 'Preview stopped', exact: true })).toHaveCount(
      0,
    );

    upgrade.release();
    await upgrade.replaced();
    await expect(page.locator('.status.live .status-label')).toHaveText('Live', {
      timeout: 120_000,
    });

    // It reads: a CLI write after the upgrade reaches the open tab.
    run(
      world,
      world.binaries.colab,
      ['page', 'write', created.pageId, '--file', '-', '--json'],
      '<h1 id="h">After</h1>',
    );
    await expect(page.frameLocator('iframe').locator('#h')).toHaveText('After', {
      timeout: 60_000,
    });

    // It saves: the browser's edit lands once.
    const box = await openSource(page);
    await expect(box).toHaveValue('<h1 id="h">After</h1>');
    await box.fill('<h1 id="h">Edited</h1>');
    await save(page);
    const read = () =>
      (
        JSON.parse(
          run(world, world.binaries.colab, ['page', 'read', created.pageId, '--json']),
        ) as {
          source: string;
        }
      ).source;
    await expect.poll(read, { timeout: 60_000 }).toBe('<h1 id="h">Edited</h1>');

    // It talks: the composer reaches the agent with the exact text, once.
    const composed = await composeChat(page, agent, 'Back after the upgrade?');
    await sendChat(page, composed);
    await expect
      .poll(
        () => {
          try {
            return composed.delivered();
          } catch {
            return '';
          }
        },
        { timeout: 60_000 },
      )
      .toContain('Back after the upgrade?');
    expect(agent.received()).toHaveLength(1);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      1,
    );
    expect(navigations).toEqual([]);
  });
});

// The wait is only for a device Remote still vouches for: every retry opens a fresh Remote Session
// first, so a grant revoked while the serve is down still ends the page with the access card.
test('a device revoked while the serve is being upgraded still ends with the access card', async () => {
  test.setTimeout(300_000);
  await withWorld(async (world) => {
    const upgrade = await attachedUpgrade(world);
    const device = await pairBrowser(world, 'restart-revoked');
    const created = createPage(world, 'Revoked in upgrade', '<h1 id="h">Revoked</h1>');
    const page = await openPage({ address: upgrade.address }, device, created);
    await expect(page.locator('.status.live .status-label')).toHaveText('Live');

    upgrade.install();
    await expect(status(page)).toContainText('Reconnecting…', { timeout: 60_000 });
    run(world, world.binaries.remote, [
      'devices',
      'revoke',
      clientId(world, 'restart-revoked'),
      '--json',
    ]);

    const card = page.locator('.tmt-ui-notice');
    await expect(card).toContainText(text.failurePairingRemoved, { timeout: 90_000 });
    await expect(card).toContainText('Pair again');
    upgrade.release();
  });
});
