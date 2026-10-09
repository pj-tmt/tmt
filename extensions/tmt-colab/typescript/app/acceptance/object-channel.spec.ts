import fs from 'node:fs';
import os from 'node:os';
import { execFileSync } from 'node:child_process';
import { build } from 'vite-plus';
import path from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { createPage, openPage, run } from './harness/ask.js';
import { pairBrowser, restartColab, restartRemote, startDoor } from './harness/browser.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

// Shipped binaries and their actual discovery/handshake, never a transport stand-in.
// This is the activation seam; upload/publication acceptance is a separate scenario.
test.afterEach(disposeActiveWorlds);

test('object channel: first demand and replacement use live discovery', async ({
  browserName,
}, info) => {
  expect(browserName).toBe('chromium');
  await withWorld(async (world) => {
    await info.attach('world-root.txt', { body: world.root, contentType: 'text/plain' });
    world.linkExtensions();
    const discovery = () => {
      const file = path.join(world.barrierDirectory, 'discovery.jsonl');
      return fs.existsSync(file)
        ? fs
            .readFileSync(file, 'utf8')
            .trim()
            .split('\n')
            .map(
              (line) =>
                JSON.parse(line) as {
                  elapsedMs: number;
                  code: number;
                  signal: string | null;
                },
            )
        : [];
    };
    const status = () =>
      JSON.parse(run(world, world.binaries.remote, ['status', '--objects', '--json'])) as {
        objectChannels: { extension: string; state: string }[];
      };
    let firstDemand: number | undefined;
    try {
      const door = await startDoor(world);
      expect(status().objectChannels).toContainEqual({
        extension: 'colab',
        state: 'unavailable',
        reason: 'setup',
      });
      const created = createPage(world, 'Storage route', '<p id="storage-route">Routed source</p>');
      const browser = await pairBrowser(world, 'storage-owner');
      const before = discovery().length;
      firstDemand = before;
      const page = await openPage(door, browser, created);
      await expect(page.frameLocator('iframe').locator('#storage-route')).toHaveText(
        'Routed source',
      );
      // No success after a hidden retry: one demand setup, already ready when sync serves data.
      expect(discovery().length - before).toBe(1);
      expect(status().objectChannels).toContainEqual({ extension: 'colab', state: 'ready' });
      const first = discovery().at(-1)!;
      expect(first.code).toBe(0);
      expect(first.signal).toBeNull();
      // The Remote handshake itself enforces 250 ms. This measures its live discovery child,
      // not OS cold first-exec qualification, and does not renew that product deadline.
      expect(first.elapsedMs).toBeLessThan(250);

      await page.close();
      await restartColab(world, door);
      const replacementBefore = discovery().length;
      const replacement = await openPage(door, browser, created);
      await expect(replacement.frameLocator('iframe').locator('#storage-route')).toHaveText(
        'Routed source',
      );
      expect(discovery().length - replacementBefore).toBe(1);
      expect(status().objectChannels).toContainEqual({ extension: 'colab', state: 'ready' });
    } finally {
      await info.attach('discovery-timing.json', {
        body: JSON.stringify(
          {
            cold: discovery()[0],
            firstDemand: firstDemand === undefined ? null : discovery()[firstDemand],
            warm: discovery().at(-1),
            observations: discovery(),
            qualification:
              'Fresh discovery processes in one isolated world; not Darwin assessment proof',
          },
          null,
          2,
        ),
        contentType: 'application/json',
      });
    }
  });
});

// Only this test client is compiled here. No product bundle or binary is replaced.
// It drives the real browser registration, Connection, crypto, fold and Writer.
test('object channel: upload original recovers, commits and publishes through real owners', async ({
  browserName,
}, info) => {
  expect(browserName).toBe('chromium');
  const fixtureRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'colab-storage-client-'));
  try {
    await build({
      configFile: false,
      root: path.resolve(import.meta.dirname, '..'),
      base: './',
      worker: { format: 'es' },
      build: {
        outDir: fixtureRoot,
        emptyOutDir: true,
        lib: {
          entry: path.join(import.meta.dirname, 'harness/storage-browser.ts'),
          formats: ['es'],
          fileName: 'storage-proof',
        },
        rollupOptions: {
          input: path.join(import.meta.dirname, 'harness/storage-browser.ts'),
          output: { entryFileNames: 'storage-proof.js' },
        },
      },
    });
    await withWorld(async (world) => {
      await info.attach('world-root.txt', { body: world.root, contentType: 'text/plain' });
      world.linkExtensions();
      const door = await startDoor(world);
      const created = createPage(world, 'Storage upload', '<p>Original upload source</p>');
      const browser = await pairBrowser(world, 'storage-upload-owner');
      const call = await storageClient(browser.page, `${door.mounts}colab/`, fixtureRoot);
      const ledger = () =>
        execFileSync(
          'sqlite3',
          [path.join(world.dataRoot, 'remote', 'objects.db'), 'SELECT count(*) FROM intents'],
          { encoding: 'utf8' },
        ).trim();
      const before = ledger();
      const connected = (await call('connect', `${door.mounts}colab/`, created.pageId)) as {
        source: string;
        device: string;
        epoch: string;
      };
      expect(connected.source).toBe('<p>Original upload source</p>');
      // Configuration names the actual backend and its effective bounds, not a default or a guess.
      const config = (await call('config')) as {
        ok: {
          result: string;
          backend: { id: string; source: string; editable: boolean };
          capabilities: Record<string, boolean>;
          limits: Record<string, unknown>;
        };
      };
      expect(config.ok).toMatchObject({
        result: 'config',
        backend: { id: 'local-fs', source: 'default', editable: false },
        capabilities: { immutableCreate: true, chunkedRead: true, recoverByOriginalId: true },
      });
      expect(config.ok.limits).toEqual({ payloadBytes: 12 * 1024 * 1024, chunkBytes: 32 * 1024 });
      const original = (await call('capture')) as {
        transferId: string;
        bytes: number;
        parts: number;
        digest: string;
        plaintextDigest: string;
      };
      expect(original.parts).toBeGreaterThan(2);
      expect(await call('prematureRead')).toBe('refused');
      expect(ledger()).toBe(before);
      expect(await call('begin')).toMatchObject({
        ok: { result: 'pending', nextIndex: 0, received: 0 },
      });
      expect(Number(ledger())).toBe(Number(before) + 1);
      expect(await call('part', 0)).toMatchObject({
        ok: { result: 'progress', nextIndex: 1, received: 32768 },
      });
      expect(await call('prematureRead')).toBe('refused');
      // No replacement ID and no replay: destroy both servers, then observe this same original.
      await call('disconnect');
      await restartRemote(world, door);
      await restartColab(world, door);
      const recovered = (await call('connect', `${door.mounts}colab/`, created.pageId)) as {
        device: string;
      };
      expect(recovered.device).toBe(connected.device);
      expect(await call('status')).toMatchObject({
        ok: { result: 'pending', nextIndex: 1, received: 32768 },
      });
      for (let index = 1; index < original.parts; index++)
        expect(await call('part', index)).toMatchObject({
          ok: { result: 'progress', nextIndex: index + 1 },
        });
      expect(await call('commit')).toMatchObject({
        ok: { result: 'committed', payloadSha256: original.digest, payloadBytes: original.bytes },
      });
      expect(await call('prematureRead')).toBe('refused');
      expect(await call('publish')).toMatchObject({ digest: original.plaintextDigest });
      expect(await call('read')).toMatchObject({ digest: original.plaintextDigest });
      expect(Number(ledger())).toBe(Number(before) + 1);
      const ownUpdateBytes = Number(
        execFileSync(
          'sqlite3',
          [
            path.join(world.dataRoot, 'colab', 'space.db'),
            "SELECT COALESCE(max(length(payload)),0) FROM receipts WHERE namespace='own'",
          ],
          { encoding: 'utf8' },
        ).trim(),
      );
      expect(ownUpdateBytes).toBeGreaterThan(0);
      expect(ownUpdateBytes).toBeLessThan(32768);
      const other = await pairBrowser(world, 'storage-read-owner');
      const otherCall = await storageClient(other.page, `${door.mounts}colab/`, fixtureRoot);
      const binding = await call('binding');
      await otherCall('connect', `${door.mounts}colab/`, created.pageId);
      await otherCall('reference', binding);
      expect(await otherCall('read')).toMatchObject({ digest: original.plaintextDigest });
      // Widening keeps the epoch; the subsequent narrowing rotates it. The old message
      // must then pass through the detached historical owner.
      run(world, world.binaries.colab, [
        'share',
        'mode',
        created.pageId,
        'link',
        '--yes',
        '--json',
      ]);
      run(world, world.binaries.colab, [
        'share',
        'mode',
        created.pageId,
        'private',
        '--yes',
        '--json',
      ]);
      const rotated = (await otherCall('connect', `${door.mounts}colab/`, created.pageId)) as {
        epoch: string;
      };
      expect(rotated.epoch).not.toBe(connected.epoch);
      expect(await otherCall('read')).toMatchObject({ digest: original.plaintextDigest });
      // Establish a current creator connection so the refusal below cannot be
      // satisfied by the earlier sharing rotation's already-closed connection.
      await call('connect', `${door.mounts}colab/`, created.pageId);
      const devices = JSON.parse(run(world, world.binaries.remote, ['devices', '--json'])) as {
        devices: { name: string; clientId: string }[];
      };
      const device = devices.devices.find((value) => value.name === browser.name);
      expect(device).toBeDefined();
      run(world, world.binaries.remote, ['devices', 'revoke', device!.clientId, '--json']);
      await expect(call('read')).rejects.toThrow();
      // Device revocation rotates affected pages too. The surviving owner must
      // establish the new epoch before reading the exact historical reference.
      const afterRevoke = (await otherCall('connect', `${door.mounts}colab/`, created.pageId)) as {
        epoch: string;
      };
      expect(afterRevoke.epoch).not.toBe(rotated.epoch);
      expect(await otherCall('read')).toMatchObject({ digest: original.plaintextDigest });
      await otherCall('disconnect');
      await info.attach('original-route.json', {
        body: JSON.stringify(
          {
            transferId: original.transferId,
            parts: original.parts,
            payloadBytes: original.bytes,
            digest: original.digest,
            recoveredDevice: recovered.device,
            ledgerIntents: ledger(),
            ownUpdateBytes,
          },
          null,
          2,
        ),
        contentType: 'application/json',
      });
    });
  } finally {
    fs.rmSync(fixtureRoot, { recursive: true });
  }
});

async function storageClient(page: Page, mount: string, fixtureRoot: string) {
  const moduleUrl = `${mount}__storage-proof/storage-proof.js`;
  await page.route(`${mount}__storage-proof/**`, async (route) => {
    const relative = new URL(route.request().url()).pathname.split('/__storage-proof/')[1];
    const file = relative && path.resolve(fixtureRoot, relative);
    if (!file || !file.startsWith(`${fixtureRoot}/`) || !fs.existsSync(file)) {
      await route.abort();
      return;
    }
    await route.fulfill({ path: file, contentType: 'text/javascript' });
  });
  // Keep the real served document and its local-network/CSP provenance. The
  // fixture client owns this tab; prevent a second app Connection/Writer.
  await page.route(`${mount}assets/index-*.js`, (route) => route.abort());
  const served = await page.goto(`${mount}`);
  expect(served?.status()).toBe(200);
  expect(served?.headers()['content-security-policy']).toContain("connect-src 'self'");
  return async (method: string, ...args: unknown[]) =>
    page.evaluate(
      async ({ url, method, args }) => {
        const client = (await import(/* @vite-ignore */ url)) as Record<
          string,
          (...values: unknown[]) => unknown
        >;
        return client[method](...args);
      },
      { url: moduleUrl, method, args },
    );
}
