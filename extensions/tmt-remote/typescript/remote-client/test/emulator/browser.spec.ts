import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import source from './colab-source.json' with { type: 'json' };
import type { start } from './browser-entry.js';
import type { SyncValue } from '../../src/sealed-sync.js';

type Client = Awaited<ReturnType<typeof start>>;
interface World {
  client: Client;
  interest: Awaited<ReturnType<Client['listen']>>;
  pending?: Promise<{ kind: string }>;
}
const origin = 'https://demo-tmt-colab.web.app';
const project = 'demo-tmt-colab';
const blessed = source.head;
const config = {
  apiKey: 'emulator-public-key',
  appId: '1:123:web:emulator',
  authDomain: `${project}.firebaseapp.com`,
  projectId: project,
};
const authPort = Number(process.env.REMOTE_SYNC_AUTH_PORT ?? 9099);
const firestorePort = Number(process.env.REMOTE_SYNC_FIRESTORE_PORT ?? 8085);
const documents = `http://127.0.0.1:${firestorePort}/v1/projects/${project}/databases/(default)/documents/x/colab`;
const space = 'a'.repeat(32),
  pageId = '10000000-0000-4000-8000-000000000001';
const pageKey = `${space}_${pageId}`;
const stream = '20000000-0000-4000-8000-000000000001';
const fields = (row: Record<string, string | number | null>) =>
  Object.fromEntries(
    Object.entries(row).map(([key, value]) => [
      key,
      value === null
        ? { nullValue: null }
        : typeof value === 'number'
          ? { integerValue: String(value) }
          : { stringValue: value },
    ]),
  );
const admin = async (path: string, data: Record<string, SyncValue>) => {
  const response = await fetch(`${documents}/${path}`, {
    method: 'PATCH',
    headers: { Authorization: 'Bearer owner', 'Content-Type': 'application/json' },
    body: JSON.stringify({ fields: data }),
  });
  expect(response.status).toBe(200);
};

test('qualified Colab member: page push hints plus REST read/query/create/put/delete and cancellation', async ({
  page,
  context,
}) => {
  if (!process.env.REMOTE_SYNC_COLAB_ARTIFACTS || process.env.REMOTE_SYNC_COLAB_HEAD !== blessed)
    throw Error('exact Colab-accepted artifact/head required; no fixture substitute or skip');
  const artifact = JSON.parse(
    await readFile(`${process.env.REMOTE_SYNC_COLAB_ARTIFACTS}/deploy-declaration-v1.json`, 'utf8'),
  ) as { declaration: string; declarationDigest: string; artifact: string };
  expect(createHash('sha256').update(artifact.declaration).digest('hex')).toBe(
    artifact.declarationDigest,
  );
  const declaration = { bytes: artifact.declaration, digest: artifact.declarationDigest };
  const unsigned = { kind: 'firestore', publicWebConfig: config, version: 1 };
  const checksum = createHash('sha256').update(JSON.stringify(unsigned)).digest('hex');
  const link = `${origin}/pair#${Buffer.from(JSON.stringify({ checksum, ...unsigned })).toString('base64url')}`;
  await page.route(`${origin}/**`, async (route) => {
    if (new URL(route.request().url()).pathname === '/test/browser-entry.js')
      return route.fulfill({
        contentType: 'text/javascript',
        body: await readFile(
          new URL('../../dist/emulator/browser-entry.js', import.meta.url),
          'utf8',
        ),
      });
    return route.fulfill({
      contentType: 'text/html',
      body: '<!doctype html><title>Private emulator world</title>',
    });
  });
  // Hosted HTTPS -> loopback emulators needs Chromium's test-only local-network permission.
  await context.grantPermissions(['local-network-access'], { origin });
  await page.goto(origin);
  const uid = await page.evaluate(
    async ({ config, declaration, link, authPort, firestorePort }) => {
      const { start } = (await import('/test/browser-entry.js')) as {
        start: typeof import('./browser-entry.js').start;
      };
      const client = await start(config, declaration, link, { authPort, firestorePort });
      (window as unknown as World).client = client;
      return client.uid;
    },
    { config, declaration, link, authPort, firestorePort },
  );
  const future = {
    timestampValue: new Date(Math.floor(Date.now() / 1000) * 1000 + 86400123).toISOString(),
  } as const;
  // Fixture-only admission by emulator Admin: Colab owns admission, not the client.
  await admin(`spaces/${space}`, fields({ space, owner: uid }));
  await admin(`pages/${pageKey}`, {
    ...fields({ space, page: pageId, epoch: 1, state: 'open' }),
    expiresAt: future,
  });
  await admin(
    `members/${pageKey}_${uid}`,
    fields({ space, page: pageId, uid, role: 'editor', link: null, token: null }),
  );
  await page.evaluate(
    async ({ space, pageId }) => {
      const w = window as unknown as World;
      await w.client.bind(space);
      w.interest = await w.client.listen({
        resource: 'log',
        scope: {
          space: { stringValue: space },
          page: { stringValue: pageId },
          epoch: { integerValue: '1' },
        },
      });
    },
    { space, pageId },
  );
  expect(await page.evaluate(() => (window as unknown as World).interest.next())).toEqual({
    kind: 'changed',
  });
  await page.evaluate(() => {
    const w = window as unknown as World;
    w.pending = w.interest.next();
  });
  const firstId = `${pageKey}_1_${stream}_1`;
  const entry = {
    ...fields({
      space,
      page: pageId,
      epoch: 1,
      stream,
      seq: 1,
      uid,
      hash: 'h'.repeat(43),
      envelope: 'ZW52ZWxvcGU',
    }),
    expiresAt: future,
  };
  expect(
    await page.evaluate(
      async ({ firstId, entry }) =>
        (window as unknown as World).client.write('create', 'log', firstId, entry),
      { firstId, entry },
    ),
  ).toEqual({ kind: 'confirmed' });
  expect(await page.evaluate(() => (window as unknown as World).pending!)).toEqual({
    kind: 'changed',
  });
  const read = await page.evaluate(
    async (firstId) => (window as unknown as World).client.read('log', firstId),
    firstId,
  );
  expect(JSON.parse(read!.bytes)).toEqual(entry);
  const query = {
    shape: 'stream' as const,
    query: {
      resource: 'log',
      stream,
      scope: {
        space: { stringValue: space },
        page: { stringValue: pageId },
        epoch: { integerValue: '1' },
        stream: { stringValue: stream },
      },
    },
  };
  expect(
    await page.evaluate((q) => (window as unknown as World).client.query(q), query),
  ).toMatchObject({ shape: 'stream', page: { cursor: { stream, seq: 1 } } });
  // Consume any coalesced hint, then create a later same-stream row and a second writer.
  await page.evaluate(() => {
    const w = window as unknown as World;
    w.pending = w.interest.next();
  });
  const secondId = `${pageKey}_1_${stream}_2`;
  expect(
    await page.evaluate(
      async ({ id, fields }) =>
        (window as unknown as World).client.write('create', 'log', id, fields),
      { id: secondId, fields: { ...entry, seq: { integerValue: '2' } } },
    ),
  ).toEqual({ kind: 'confirmed' });
  expect(await page.evaluate(() => (window as unknown as World).pending!)).toEqual({
    kind: 'changed',
  });
  await page.evaluate(() => {
    const w = window as unknown as World;
    w.pending = w.interest.next();
  });
  const newStream = '20000000-0000-4000-8000-000000000002';
  expect(
    await page.evaluate(
      async ({ id, fields }) =>
        (window as unknown as World).client.write('create', 'log', id, fields),
      {
        id: `${pageKey}_1_${newStream}_1`,
        fields: { ...entry, stream: { stringValue: newStream } },
      },
    ),
  ).toEqual({ kind: 'confirmed' });
  expect(await page.evaluate(() => (window as unknown as World).pending!)).toEqual({
    kind: 'changed',
  });
  // Owner whole-document admission mode uses the same single-target commit, not a second stack.
  const pageFields = {
    ...fields({ space, page: pageId, epoch: 2, state: 'open' }),
    expiresAt: future,
  };
  expect(
    await page.evaluate(
      async ({ id, fields }) =>
        (window as unknown as World).client.write('put', 'pages', id, fields),
      { id: pageKey, fields: pageFields },
    ),
  ).toEqual({ kind: 'confirmed' });
  const linkId = '30000000-0000-4000-8000-000000000001';
  const rowId = `${pageKey}_${linkId}`;
  const linkFields = fields({
    space,
    page: pageId,
    link: linkId,
    role: 'viewer',
    token: 'A'.repeat(43),
  });
  expect(
    await page.evaluate(
      async ({ id, fields }) =>
        (window as unknown as World).client.write('create', 'links', id, fields),
      { id: rowId, fields: linkFields },
    ),
  ).toEqual({ kind: 'confirmed' });
  expect(
    await page.evaluate(
      async (id) => (window as unknown as World).client.remove('links', id),
      rowId,
    ),
  ).toEqual({ kind: 'confirmed' });
  expect(
    await page.evaluate(
      async (id) => (window as unknown as World).client.cancelledRead('log', id),
      firstId,
    ),
  ).toBe('REMOTE_STATE_UNAVAILABLE');
  await page.evaluate(async () => {
    const w = window as unknown as World;
    w.interest.close();
    await w.client.signOut();
    await w.client.close();
  });
  console.log(
    `Qualified Colab ${blessed}: anonymous Auth; member page-scope listener; empty/new/later-writer hints; REST read/query/create/put/delete`,
  );
});
