import { randomBytes, randomUUID } from 'node:crypto';
import { request } from 'node:http';
import {
  binary,
  decodeHeader,
  deriveSpaceId,
  digest,
  encodeBinary,
  Envelope,
  MAX_ENVELOPE_JSON,
  payload,
  statement,
  strictJson,
  text,
} from '@tmt/colab-client';
import { execFileSync } from 'node:child_process';
import { mkdir } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { openReaderLink, startDoor } from './harness/browser.js';
import { createPage, freePort, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';
import type { AcceptanceWorld } from './harness/world.js';

// #1545 read-only share link. The page, the link and Reset are the real owner commands; the
// reader is a Chromium profile that never paired with the door.

test.afterEach(disposeActiveWorlds);

const colab = (world: AcceptanceWorld, args: string[], input?: string) =>
  JSON.parse(run(world, world.binaries.colab, [...args, '--json'], input)) as Record<
    string,
    unknown
  >;

/** Link-device rows in the owner store: one per link, however often it is opened. */
const deviceRows = (world: AcceptanceWorld) =>
  Number(
    execFileSync(
      'sqlite3',
      [path.join(world.dataRoot, 'colab', 'space.db'), 'SELECT count(*) FROM devices'],
      { encoding: 'utf8' },
    ).trim(),
  );

/** The seed is the one secret in a reader link; it lives in the fragment only. */
const seedOf = (readerPath: string) => new URLSearchParams(readerPath.split('#')[1]).get('seed')!;

test('reader link: opens unpaired, shows live edits read-only, and ends on Reset', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const longBody = Array.from(
      { length: 48 },
      (_, index) =>
        `<p style="min-height:54px">Reader paragraph ${index + 1}. The browser window scrolls beneath the Colab bar.</p>`,
    ).join('');
    const created = createPage(
      world,
      'Reader acceptance',
      `<h1 id="text">First text</h1><a href="#target">Jump to reader anchor</a>${longBody}<h2 id="target">Reader anchor</h2><div style="height:900px"></div>`,
    );
    colab(world, ['share', 'mode', created.pageId, 'link', '--yes']);
    const added = colab(world, ['share', 'link', 'add', created.pageId, '--yes']);
    const readerPath = added.readerPath as string;
    expect(readerPath.startsWith('x/colab/read#v=1&')).toBe(true);

    // An unpaired profile gets the reader entry but no owner file.
    const owner = await fetch(`${door.address}/x/colab/index.html`);
    expect(owner.status).toBe(403);

    const baseline = deviceRows(world);
    const reader = await openReaderLink(world, door, readerPath, 'reader-one');
    await expect(reader.page).toHaveTitle('Colab');
    const frame = reader.page.frameLocator('iframe');
    await expect(frame.locator('#text')).toHaveText('First text', { timeout: 30_000 });
    const element = reader.page.locator('iframe');
    await expect.poll(async () => (await element.boundingBox())?.height ?? 0).toBeGreaterThan(3000);
    await expect(element).toHaveAttribute('scrolling', 'no');
    await expect
      .poll(() =>
        frame.locator('html').evaluate((node) => node.scrollHeight <= node.clientHeight + 1),
      )
      .toBe(true);
    const captures = process.env.COLAB_READER_CAPTURE_DIR;
    if (captures) await mkdir(captures, { recursive: true });
    for (const width of [1440, 390]) {
      await reader.page.setViewportSize({ width, height: 900 });
      for (const theme of ['light', 'dark'] as const) {
        await reader.page.evaluate(
          (value) => (document.documentElement.dataset.theme = value),
          theme,
        );
        await expect
          .poll(async () => (await element.boundingBox())?.height ?? 0)
          .toBeGreaterThan(3000);
        const frameBox = await element.boundingBox();
        const contentWidth = await reader.page.evaluate(() => document.documentElement.clientWidth);
        expect(frameBox?.x).toBe(0);
        expect(Math.abs((frameBox?.width ?? 0) - contentWidth)).toBeLessThan(1);
        await reader.page.evaluate(() => window.scrollTo(0, 0));
        await expect.poll(() => reader.page.evaluate(() => window.scrollY)).toBe(0);
        await expect(reader.page.locator('.tmt-ui-header')).toBeInViewport();
        if (captures)
          await reader.page.screenshot({ path: `${captures}/reader-${width}-${theme}-top.png` });
        await reader.page.evaluate(() => window.scrollTo(0, 1800));
        await expect.poll(() => reader.page.evaluate(() => window.scrollY)).toBeGreaterThan(1000);
        await expect(reader.page.locator('.tmt-ui-header')).toBeInViewport();
        expect(
          Math.abs((await reader.page.locator('.tmt-ui-header').boundingBox())?.y ?? Infinity),
        ).toBeLessThan(1);
        expect(
          await frame.locator('html').evaluate((node) => node.ownerDocument.defaultView!.scrollY),
        ).toBe(0);
        if (captures)
          await reader.page.screenshot({
            path: `${captures}/reader-${width}-${theme}-scrolled.png`,
          });
      }
    }
    await reader.page.setViewportSize({ width: 1280, height: 900 });
    await reader.page.evaluate(() => {
      document.documentElement.dataset.theme = 'light';
      window.scrollTo(0, 0);
    });
    const frameWidth = (await element.boundingBox())?.width;
    const pageHeight = await reader.page.evaluate(() => document.documentElement.scrollHeight);
    await reader.page.getByLabel('Info', { exact: true }).click();
    await expect(reader.page.getByText('You are reading a shared page.')).toBeVisible();
    expect((await element.boundingBox())?.width).toBe(frameWidth);
    expect(await reader.page.evaluate(() => document.documentElement.scrollHeight)).toBe(
      pageHeight,
    );
    await reader.page.getByLabel('Info', { exact: true }).click();
    await frame.getByRole('link', { name: 'Jump to reader anchor' }).click();
    await expect.poll(() => reader.page.evaluate(() => window.scrollY)).toBeGreaterThan(2000);
    expect(
      await frame.locator('html').evaluate((node) => node.ownerDocument.defaultView!.scrollY),
    ).toBe(0);
    await reader.page.evaluate(() => window.scrollTo(0, 0));
    // The fragment left the address bar, and the page offers no write or Ask control.
    expect(await reader.page.evaluate(() => location.hash)).toBe('');
    await expect(reader.page.getByText('Read-only').first()).toBeVisible();
    await expect(reader.page.locator('textarea, [data-testid=ask-action]')).toHaveCount(0);

    // A live edit by the owner's agent reaches the reader.
    const read = colab(world, ['page', 'read', created.pageId]);
    colab(
      world,
      [
        'page',
        'write',
        created.pageId,
        '--file',
        '-',
        '--expected-revision',
        read.revision as string,
      ],
      '<h1 id="text">Second text</h1>',
    );
    await expect(frame.locator('#text')).toHaveText('Second text', { timeout: 30_000 });
    await expect
      .poll(async () => (await element.boundingBox())?.height ?? Infinity)
      .toBeLessThan(1100);

    // Re-opening the same link in more browsers presents the same derived device: still one row.
    for (const name of ['reader-again-1', 'reader-again-2']) {
      const again = await openReaderLink(world, door, readerPath, name);
      await expect(again.page.frameLocator('iframe').locator('#text')).toHaveText('Second text', {
        timeout: 30_000,
      });
    }
    expect(deviceRows(world)).toBe(baseline + 1);

    // The seed never left the browser: no request URL or body carries it.
    const seed = seedOf(readerPath);
    expect(
      reader.requests.filter((r) => r.url.includes(seed) || (r.body ?? '').includes(seed)),
    ).toEqual([]);

    // Reset ends the old link, live, and the replacement opens in another profile.
    const reset = colab(world, [
      'share',
      'link',
      'reset',
      created.pageId,
      added.linkId as string,
      '--yes',
    ]);
    await expect(reader.page.getByRole('heading', { name: 'Access ended', level: 2 })).toBeVisible({
      timeout: 60_000,
    });
    await expect(reader.page.locator('iframe')).toHaveCount(0);
    const stale = await openReaderLink(world, door, readerPath, 'reader-stale');
    await expect(stale.page.getByRole('heading', { name: 'Access ended', level: 2 })).toBeVisible({
      timeout: 30_000,
    });
    const fresh = await openReaderLink(world, door, reset.readerPath as string, 'reader-fresh');
    await expect(fresh.page.frameLocator('iframe').locator('#text')).toHaveText('Second text', {
      timeout: 30_000,
    });
    // The replacement link is a new identity with its own device; the old one stays revoked.
    expect(deviceRows(world)).toBe(baseline + 2);
  });
});

// Local-owner setup is needed because the UI/CLI Create link intentionally selects one page.
// It uses the same private root-authorized IPC as the CLI, never inserts store rows.
async function twoPageLink(world: AcceptanceWorld, pages: string[]) {
  const current = colab(world, ['show', pages[0]]) as {
    spaceId: string;
    membershipHead: { revision: string };
  };
  const linkId = randomUUID(),
    seed = randomBytes(32).toString('base64url');
  const operationId = randomUUID();
  const body = JSON.stringify({
    space: current.spaceId,
    page: pages[0],
    expectedRevision: current.membershipHead.revision,
    operationId,
    operation: 'link.add',
    payload: Buffer.from(
      JSON.stringify({ linkId, role: 'viewer', pages: [...pages].sort(), seed }),
    ).toString('base64url'),
  });
  const ack = await new Promise<{
    operationId: string;
    membershipHead: { revision: string; statementHash: string };
  }>((resolve, reject) => {
    const req = request(
      {
        socketPath: path.join(world.dataRoot, 'colab', 'door.sock'),
        path: '/.tmt/colab/management',
        method: 'POST',
        headers: {
          Host: 'localhost',
          'Content-Length': Buffer.byteLength(body),
          Connection: 'close',
        },
      },
      (response) => {
        let data = '';
        response.setEncoding('utf8');
        response.on('data', (part: string) => {
          data += part;
          if (data.length > 8192) req.destroy(new Error('Owner IPC acknowledgment exceeds bound'));
        });
        response.on('end', () => {
          if (response.statusCode !== 200)
            reject(new Error(`Owner IPC denied: ${response.statusCode} ${data}`));
          else resolve(JSON.parse(data));
        });
        response.on('error', reject);
      },
    );
    req.setTimeout(4000, () => req.destroy(new Error('Owner IPC deadline exceeded')));
    req.on('error', reject);
    req.end(body);
  });
  expect(ack.operationId).toBe(operationId);
  return {
    linkId,
    readerPath: `x/colab/read#v=1&space=${current.spaceId}&page=${pages[0]}&link=${linkId}&rev=${ack.membershipHead.revision}&st=${ack.membershipHead.statementHash}&seed=${seed}`,
  };
}

/** Drive the shipped public ticket transport in an unpaired profile; no new public UI. */
async function publicRead(
  world: AcceptanceWorld,
  door: Awaited<ReturnType<typeof startDoor>>,
  pageId: string,
  source: string,
  name: string,
) {
  const browser = await openReaderLink(world, door, 'x/colab/read', name);
  const { spaceId: space } = colab(world, ['show', pageId]) as { spaceId: string };
  const admitted = await browser.page.evaluate(
    async ({ mount, space, page }) => {
      const post = async (path: string, body: unknown) => {
        const response = await fetch(new URL(path, mount), {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
          signal: AbortSignal.timeout(10000),
        });
        if (response.status !== 200)
          throw new Error(`Public admission rejected: ${response.status}`);
        return response.json();
      };
      const challenge = await post('api/readers/challenge', { kind: 'public', space, page });
      const session = await post('api/readers/session', {
        kind: 'public',
        challengeId: challenge.challengeId,
      });
      const url = new URL('sync', mount);
      url.protocol = 'ws:';
      const socket = new WebSocket(url, ['colab-sync-v1', `colab-reader-v1.${session.token}`]);
      const state = { closed: false, denial: '', socket, session };
      Object.assign(window, { publicReader: state });
      const frames: Record<string, unknown>[] = [];
      await new Promise<void>((resolve, reject) => {
        const deadline = setTimeout(
          () => reject(new Error('Public catchup deadline exceeded')),
          10000,
        );
        socket.onopen = () =>
          socket.send(
            JSON.stringify({
              version: 1,
              type: 'hello',
              space,
              page,
              epoch: session.epoch,
              device: session.principal,
              membershipRevision: '0',
              cursors: [],
            }),
          );
        socket.onerror = () => {
          clearTimeout(deadline);
          reject(new Error('Public socket failed'));
        };
        socket.onclose = () => {
          state.closed = true;
          clearTimeout(deadline);
          reject(new Error('Public socket ended before catchup'));
        };
        socket.onmessage = (event) => {
          const frame = JSON.parse(event.data);
          if (frame.type === 'error') {
            state.denial = frame.code;
            clearTimeout(deadline);
            reject(new Error(`Public catchup: ${frame.code}`));
            return;
          }
          if (frame.type !== 'catchup' || frames.length >= 32) {
            clearTimeout(deadline);
            reject(new Error('Unexpected public frame'));
            return;
          }
          frames.push(frame);
          if (frame.more)
            socket.send(
              JSON.stringify({
                version: 1,
                type: 'ack',
                space,
                page,
                epoch: session.epoch,
                cursors: [],
              }),
            );
          else {
            clearTimeout(deadline);
            socket.onmessage = (event) => {
              const value = JSON.parse(event.data);
              if (value.type === 'error') state.denial = value.code;
            };
            socket.onclose = () => {
              state.closed = true;
            };
            resolve();
          }
        };
      });
      return {
        frames,
        session: {
          space: session.space,
          page: session.page,
          epoch: session.epoch,
          ownerKey: session.ownerKey,
        },
      };
    },
    { mount: `${door.address}/x/colab/`, space, page: pageId },
  );
  // Native public-key publication is admitted by the shared browser codecs before any read.
  const owner = binary(admitted.session.ownerKey, 32, 32);
  expect(await deriveSpaceId(owner)).toBe(space);
  let head: statement.Head | null = null;
  let root: Uint8Array | undefined;
  let signedBaseline: payload.Baseline | undefined;
  for (const frame of admitted.frames) {
    expect(frame).toMatchObject({
      version: 1,
      type: 'catchup',
      space,
      page: pageId,
      epoch: admitted.session.epoch,
    });
    const membership = (frame.membershipHead ?? frame.membership) as
      | { statements: string[] }
      | undefined;
    for (const raw of membership?.statements ?? []) {
      const verified = await statement.Envelope.fromJson(
        binary(raw, payload.MAX_BYTES * 2),
      ).verifyNext(space, owner, head);
      head = verified.head;
      const p = verified.payload;
      if (
        p.operation === 'epoch.advance' &&
        p.value.pageId === pageId &&
        p.value.epoch === admitted.session.epoch
      )
        signedBaseline = p.value.baseline;
      if (p.operation === 'page.share' && p.value.pageId === pageId && p.value.mode === 'public') {
        const key = p.value.publishedKeys?.find((key) => key.epoch === admitted.session.epoch);
        if (key) root = binary(key.key, 32, 32);
      }
    }
    if (frame.wraps) expect(frame.wraps).toEqual([]);
  }
  expect(head).not.toBeNull();
  expect(root).toBeDefined();
  expect(signedBaseline).toBeDefined();
  const first = admitted.frames[0] as unknown as {
    membershipHead: { revision: string; statementHash: string };
    baseline: string;
    baselineObject: { envelope: string; envelopeHash: string };
  };
  expect(String(head!.revision)).toBe(first.membershipHead.revision);
  expect(encodeBinary(head!.hash)).toBe(first.membershipHead.statementHash);
  const baseline = payload.decodeBaseline(binary(first.baseline, 8192));
  expect(baseline).toEqual(signedBaseline);
  const envelope = Envelope.fromJson(binary(first.baselineObject.envelope, MAX_ENVELOPE_JSON));
  const hash = encodeBinary(await envelope.hash());
  expect(hash).toBe(first.baselineObject.envelopeHash);
  expect(hash).toBe(baseline.objectEnvelopeHash);
  const context = decodeHeader(envelope.header()).context;
  expect(context).toMatchObject({
    space,
    page: pageId,
    epoch: admitted.session.epoch,
    kind: 'html',
    namespace: 'content',
    membershipRevision: baseline.membershipRevision,
    authorDevice: head!.ownerMember.id,
  });
  const plain = strictJson(
    await envelope.open(context, root!, head!.ownerMember.signingKey),
    16 * 1024 * 1024,
  ) as {
    source: string;
    update: string;
  };
  expect(plain.source).toBe(source);
  expect(encodeBinary(await digest(text(source)))).toBe(baseline.sourceDigest);
  return browser;
}

test('public transport is read-only and both narrowing paths revoke two-page links after scoped Reset', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const source = '<h1 id="public">Authenticated public baseline</h1>';
    const first = createPage(world, 'Public first', source);
    const second = createPage(world, 'Public second', source);
    for (const page of [first, second])
      colab(world, ['share', 'mode', page.pageId, 'link', '--yes']);
    const before = colab(world, ['page', 'read', first.pageId]);
    const link = await twoPageLink(world, [first.pageId, second.pageId]);
    const otherPath = (readerPath: string) =>
      readerPath.replace(`page=${first.pageId}`, `page=${second.pageId}`);
    const old = await openReaderLink(world, door, link.readerPath, 'multi-old-first');
    const oldOther = await openReaderLink(
      world,
      door,
      otherPath(link.readerPath),
      'multi-old-second',
    );
    for (const reader of [old, oldOther])
      await expect(reader.page.frameLocator('iframe').locator('#public')).toHaveText(
        'Authenticated public baseline',
      );
    const reset = colab(world, ['share', 'link', 'reset', first.pageId, link.linkId, '--yes']);
    for (const reader of [old, oldOther])
      await expect(
        reader.page.getByRole('heading', { name: 'Access ended', exact: true, level: 2 }),
      ).toBeVisible({ timeout: 60000 });
    const replacement = reset.readerPath as string;
    const fresh = await openReaderLink(world, door, replacement, 'multi-fresh-first');
    const freshOther = await openReaderLink(
      world,
      door,
      otherPath(replacement),
      'multi-fresh-second',
    );
    for (const reader of [fresh, freshOther])
      await expect(reader.page.frameLocator('iframe').locator('#public')).toHaveText(
        'Authenticated public baseline',
      );
    const links = colab(world, ['share', 'link', 'ls', second.pageId]).links as {
      id: string;
      pages: string[];
      revoked: boolean;
    }[];
    expect(
      links.find((l) => l.id === new URLSearchParams(replacement.split('#')[1]).get('link')),
    ).toMatchObject({
      pages: [first.pageId, second.pageId].sort(),
      revoked: false,
    });
    // Link readers require link mode. Publishing preserves the link assignment so a
    // later narrowing must revoke it globally, including the other page's live reader.
    colab(world, ['share', 'mode', first.pageId, 'public', '--yes']);
    const publicOne = await publicRead(world, door, first.pageId, source, 'public-before-private');
    for (const api of ['api/session', 'api/pages', 'api/management']) {
      expect(
        await publicOne.page.evaluate(
          async (url) =>
            (
              await fetch(url, {
                method: url.endsWith('management') ? 'POST' : 'GET',
                body: url.endsWith('management') ? '{}' : undefined,
              })
            ).status,
          `${door.address}/x/colab/${api}`,
        ),
      ).toBe(403);
    }
    const denied = await publicRead(world, door, first.pageId, source, 'public-append-denied');
    await denied.page.evaluate(() => {
      const { socket, session } = (
        window as unknown as {
          publicReader: {
            socket: WebSocket;
            session: { space: string; page: string; epoch: string; principal: string };
          };
        }
      ).publicReader;
      socket.send(
        JSON.stringify({
          version: 1,
          type: 'append',
          space: session.space,
          page: session.page,
          epoch: session.epoch,
          streamId: session.principal,
          seq: '1',
          envelopeHash: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
          envelope: { objectId: '00'.repeat(32) },
        }),
      );
    });
    await expect
      .poll(() =>
        denied.page.evaluate(
          () => (window as unknown as { publicReader: { denial: string } }).publicReader.denial,
        ),
      )
      .toBe('DENIED');
    const publicPrivate = publicOne;
    colab(world, ['share', 'mode', first.pageId, 'private', '--yes']);
    await expect
      .poll(() =>
        publicPrivate.page.evaluate(
          () => (window as unknown as { publicReader: { closed: boolean } }).publicReader.closed,
        ),
      )
      .toBe(true);
    for (const reader of [fresh, freshOther])
      await expect(
        reader.page.getByRole('heading', { name: 'Access ended', exact: true, level: 2 }),
      ).toBeVisible({ timeout: 60000 });
    const stale = await openReaderLink(world, door, otherPath(replacement), 'multi-narrowed-stale');
    await expect(
      stale.page.getByRole('heading', { name: 'Access ended', exact: true, level: 2 }),
    ).toBeVisible();
    const space = colab(world, ['show', first.pageId]).spaceId as string;
    const noPublicTicket = async (browser: typeof publicPrivate) =>
      browser.page.evaluate(
        async ({ url, space, page }) =>
          (
            await fetch(url, {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ kind: 'public', space, page }),
            })
          ).status,
        { url: `${door.address}/x/colab/api/readers/challenge`, space, page: first.pageId },
      );
    expect(await noPublicTicket(publicPrivate)).toBe(403);
    colab(world, ['share', 'mode', first.pageId, 'link', '--yes']);
    const next = await twoPageLink(world, [first.pageId, second.pageId]);
    const nextOther = await openReaderLink(
      world,
      door,
      otherPath(next.readerPath),
      'multi-next-second',
    );
    await expect(nextOther.page.frameLocator('iframe').locator('#public')).toHaveText(
      'Authenticated public baseline',
    );
    colab(world, ['share', 'mode', first.pageId, 'public', '--yes']);
    const publicLink = await publicRead(world, door, first.pageId, source, 'public-link-live');
    colab(world, ['share', 'mode', first.pageId, 'link', '--yes']);
    await expect
      .poll(() =>
        publicLink.page.evaluate(
          () => (window as unknown as { publicReader: { closed: boolean } }).publicReader.closed,
        ),
      )
      .toBe(true);
    await expect(
      nextOther.page.getByRole('heading', { name: 'Access ended', exact: true, level: 2 }),
    ).toBeVisible({ timeout: 60000 });
    expect(await noPublicTicket(publicLink)).toBe(403);
    expect((colab(world, ['show', second.pageId]).page as { sharing: string }).sharing).toBe(
      'link',
    );
    const staleNext = await openReaderLink(
      world,
      door,
      otherPath(next.readerPath),
      'multi-link-narrowed-stale',
    );
    await expect(
      staleNext.page.getByRole('heading', { name: 'Access ended', exact: true, level: 2 }),
    ).toBeVisible();
    expect(colab(world, ['page', 'read', first.pageId]).source).toBe(before.source);
    expect(world.coreCalls().filter((c) => c.operation === 'dispatch.create')).toHaveLength(0);
  });
});
