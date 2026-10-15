import { text } from '../src/strings.js';
import { chromium, expect, test } from '@playwright/test';
import { Envelope, decodeHeader } from '@tmt/colab-client';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { run } from './harness/ask.js';
import { startDoor } from './harness/browser.js';
import { composed, serve } from './harness/hosting.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

// Run only via scripts/hosted-reader-emulator.mjs. The wrapper builds the exact private native
// emulator binary and owns Auth/Firestore; this proof never selects or reaches a real project.
const project = 'demo-tmt-colab-hosted';
const host = (name: string) => {
  const value = process.env[name];
  if (!value || !/^127\.0\.0\.1:[1-9][0-9]{0,4}$/.test(value))
    throw new Error(`${name}: loopback emulator required`);
  return value;
};
const origin = `https://${project}.web.app`;
const config = {
  apiKey: 'fake-key',
  appId: '1:12345:web:proof',
  authDomain: `${project}.firebaseapp.com`,
  projectId: project,
};
function remoteLink() {
  const body = JSON.stringify({ kind: 'firestore', publicWebConfig: config, version: 1 });
  const checksum = createHash('sha256').update(body).digest('hex');
  return `${origin}/pair#${Buffer.from(JSON.stringify({ checksum, ...JSON.parse(body) })).toString('base64url')}`;
}

test.afterEach(disposeActiveWorlds);
test('a hosted editor verifies native content, publishes one own edit request and observes owner application without a local door', async () => {
  if (process.env.GCLOUD_PROJECT !== project)
    throw new Error('the demo emulator harness is required');
  const auth = host('FIREBASE_AUTH_EMULATOR_HOST'),
    firestore = host('FIRESTORE_EMULATOR_HOST');
  await withWorld(async (world) => {
    // Declaration discovery clears the child environment by contract. This proof needs the real
    // invoking Core, not Ask's barrier wrapper (whose private coordination env is deliberately
    // outside that allow-list). Match the reviewed B3 native proof without changing production.
    const environment = world.env.bind(world);
    world.env = (pane?: string) => ({ ...environment(pane), TMT_EXECUTABLE: world.binaries.tmt });
    world.linkExtensions();
    world.onDispose(async () => {
      for (const name of fs.readdirSync(world.root).filter((name) => name.endsWith('.stderr')))
        await test.info().attach(name, {
          body: fs.readFileSync(path.join(world.root, name)),
          contentType: 'text/plain',
        });
    });
    run(world, world.binaries.remote, ['add', 'proof', remoteLink(), '--json']);
    run(world, world.binaries.remote, ['settings', 'default-remote', 'proof', '--json']);
    run(world, world.binaries.remote, [
      '--remote',
      'proof',
      'sync',
      'sign-in',
      '--project',
      project,
      '--json',
    ]);
    const created = JSON.parse(
      run(
        world,
        world.binaries.colab,
        ['page', 'create', '--title', 'Hosted notes', '--file', '-', '--no-open', '--json'],
        '<main><h1>Native owner first version</h1></main>',
      ),
    ) as { pageId: string };
    run(world, world.binaries.colab, ['share', 'mode', created.pageId, 'link', '--yes', '--json']);
    // Sharing activity after serve startup selects this page, as in Colab's reviewed B3c-2 proof.
    await startDoor(world);
    // A viewer's unused device chain is not an author, even when editor link chains are valid.
    // Keep this non-writer in discovery so removing the observed-writers filter is detected.
    run(world, world.binaries.colab, [
      'share',
      'link',
      'add',
      created.pageId,
      '--role',
      'viewer',
      '--yes',
      '--json',
    ]);
    const added = JSON.parse(
      run(world, world.binaries.colab, [
        'share',
        'link',
        'add',
        created.pageId,
        '--role',
        'editor',
        '--yes',
        '--json',
      ]),
    ) as { linkId: string; readerPath: string };
    const fragment = added.readerPath.slice(added.readerPath.indexOf('#'));
    // The test only routes the link Colab actually printed; it never manufactures a root or role.
    const space = new URLSearchParams(fragment.slice(1)).get('space')!;
    const admin = async (collection: string) => {
      const response = await fetch(
        `http://${firestore}/v1/projects/${project}/databases/(default)/documents/x/colab/${collection}`,
        {
          headers: { Authorization: 'Bearer owner' },
          signal: AbortSignal.timeout(5000),
        },
      );
      expect(response.ok).toBe(true);
      return (await response.json()) as {
        documents?: Array<{ name: string; fields: Record<string, { stringValue?: string }> }>;
      };
    };
    await expect
      .poll(
        async () =>
          (await admin('links')).documents?.some((d) =>
            d.name.endsWith(`${space}_${created.pageId}_${added.linkId}`),
          ) ?? false,
        { timeout: 90000, intervals: [100, 250, 500, 1000] },
      )
      .toBe(true);
    await expect
      .poll(async () => (await admin('log')).documents?.length ?? 0, {
        timeout: 60000,
        intervals: [100, 250, 500, 1000],
      })
      .toBeGreaterThan(0);
    const initialLogDocuments = (await admin('log')).documents?.length ?? 0;
    const { view, files, manifest } = composed(world);
    const { origin: local } = await serve(world, view, files);
    const policy = manifest.files.find((file) => file.path === '/index.html')!.csp!;
    expect(policy.match(/connect-src ([^;]+)/)?.[1].split(' ')).toEqual([
      "'self'",
      'https://identitytoolkit.googleapis.com',
      'https://securetoken.googleapis.com',
      'https://firestore.googleapis.com',
    ]);
    const browser = await chromium.launch({ headless: true });
    world.onDispose(async () => {
      await browser.close();
    });
    // A fresh context has no registered worker. Blocking workers injects a Playwright shim
    // into the deliberately opaque renderer and violates its sandbox, so preserve that boundary.
    const context = await browser.newContext();
    await context.tracing.startChunk({ title: 'Hosted reader emulator' });
    world.onDispose(async () => {
      const trace = test.info().outputPath('hosted-context.zip');
      await context.tracing.stopChunk({ path: trace });
      await test.info().attach('hosted-context', { path: trace, contentType: 'application/zip' });
    });
    const unexpected: string[] = [],
      paths: string[] = [],
      writes: string[] = [];
    let phase: 'open' | 'wake' | 'edit' = 'open';
    const queries: Array<{ phase: string; collection: string; limit?: number; documents: number }> =
      [];
    const gets: Array<{ phase: string; collection: string; target: string; status: number }> = [];
    const listens: Array<{ phase: string; documentChangeMarkers: number }> = [];
    const boundaries: Record<string, unknown> = {};
    let openConverged = false,
      wakeConverged = false,
      editConverged = false;
    let pageGets = 0;
    let holderToken = '';
    await context.addInitScript(() => {
      (window as unknown as { __violations: string[] }).__violations = [];
      document.addEventListener('securitypolicyviolation', (event) =>
        (window as unknown as { __violations: string[] }).__violations.push(
          event.violatedDirective,
        ),
      );
    });
    await context.route('**/*', async (route) => {
      const request = route.request(),
        url = new URL(request.url());
      paths.push(url.origin + url.pathname);
      if (url.origin === origin) {
        if (url.pathname === '/__/firebase/init.json') await route.fulfill({ json: config });
        else
          await route.fulfill({
            response: await route.fetch({ url: `${local}${url.pathname}${url.search}` }),
          });
      } else if (
        url.origin === 'https://identitytoolkit.googleapis.com' ||
        url.origin === 'https://securetoken.googleapis.com'
      ) {
        const response = await route.fetch({
          url: `http://${auth}/${url.hostname}${url.pathname}${url.search}`,
        });
        if (url.pathname.endsWith('accounts:signUp'))
          holderToken = ((await response.json()) as { idToken: string }).idToken;
        await route.fulfill({ response });
      } else if (url.origin === 'https://firestore.googleapis.com') {
        if (url.pathname.endsWith(':commit')) writes.push(request.postData() ?? '');
        const requestedPhase = phase;
        const response = await route
          .fetch({ url: `http://${firestore}${url.pathname}${url.search}` })
          .catch(async (error: unknown) => {
            if (!(error instanceof Error) || !error.message.includes('socket hang up')) throw error;
            // An aborted long-poll must not replace the product assertion with proxy noise.
            await route.abort();
          });
        if (!response) return;
        if (request.method() === 'GET' && url.pathname.includes('/documents/'))
          gets.push({
            phase: requestedPhase,
            collection: url.pathname.split('/').at(-2)!,
            target: url.pathname.split('/').at(-1)!,
            status: response.status(),
          });
        if (url.pathname.includes('Listen/channel'))
          listens.push({
            phase,
            documentChangeMarkers: ((await response.text()).match(/"documentChange"\s*:/g) ?? [])
              .length,
          });
        if (response.ok() && url.pathname.endsWith(':runQuery')) {
          const input = request.postDataJSON() as {
            structuredQuery: { from: Array<{ collectionId: string }>; limit?: number };
          };
          const rows = (await response.json()) as Array<{ document?: unknown }>;
          queries.push({
            phase: requestedPhase,
            limit: input.structuredQuery.limit,
            collection: input.structuredQuery.from[0].collectionId,
            documents: rows.filter((row) => row.document !== undefined).length,
          });
        }
        if (response.ok() && request.method() === 'GET' && url.pathname.includes('/pages/'))
          pageGets += 1;
        await route.fulfill({ response });
      } else {
        unexpected.push(url.origin);
        await route.abort();
      }
    });
    world.onDispose(async () => {
      // WebChannel requests may still be awaiting a server response when this bounded proof ends.
      await context.unrouteAll({ behavior: 'ignoreErrors' });
    });
    const page = await context.newPage();
    // Read only the committed advisory ledger; never inspect identity or key records.
    const budget = () =>
      page.evaluate(async () => {
        const db = await new Promise<IDBDatabase>((resolve, reject) => {
          const request = indexedDB.open('tmt-colab', 1);
          request.onsuccess = () => resolve(request.result);
          request.onerror = () => reject(request.error);
        });
        try {
          return await new Promise<{ day: number; reads: number; writes: number } | undefined>(
            (resolve, reject) => {
              const tx = db.transaction('keys', 'readonly');
              const read = tx.objectStore('keys').get('hosted-reads-v1');
              tx.oncomplete = () => resolve(read.result);
              tx.onerror = () => reject(tx.error);
              tx.onabort = () => reject(tx.error);
            },
          );
        } finally {
          db.close();
        }
      });
    world.onDispose(async () => {
      boundaries.final = await budget().catch(() => ({ unavailable: true }));
      const body = JSON.stringify(
        {
          initialLogDocuments,
          openConverged,
          wakeConverged,
          editConverged,
          boundaries,
          pageGets,
          queries,
          gets,
          listens,
          admissionWrites: writes.filter((body) =>
            JSON.parse(body).writes[0].update.name.includes('/members/'),
          ).length,
          ownLogCreates: writes.filter((body) =>
            JSON.parse(body).writes[0].update.name.includes('/log/'),
          ).length,
          note: 'REST phases identify request start; Listen markers identify response delivery and are not unique documents. Ledger totals are advisory reservations, not billed reads. Admin oracles excluded. Open/wake/edit boundaries are verified rendered headings. The edit phase includes fresh authority/catch-up, one own create and one same-ID readback; owner-native writes are not browser commits.',
        },
        null,
        2,
      );
      const summary = test.info().outputPath('provider-read-summary.json');
      fs.writeFileSync(summary, body);
      await test
        .info()
        .attach('provider-read-summary', { path: summary, contentType: 'application/json' });
    });
    const crashes: string[] = [];
    page.on('pageerror', (error) => crashes.push(error.message));
    await page.goto(`${origin}/read/${created.pageId}${fragment}`);
    await expect(page).toHaveURL(`${origin}/read/${created.pageId}`);
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await expect(page.getByText('Live', { exact: true })).toBeVisible({ timeout: 25000 });
    await expect(page.locator('iframe')).toHaveCount(1);
    await expect(page.frameLocator('iframe').locator('h1')).toHaveText(
      'Native owner first version',
      { timeout: 60000 },
    );
    openConverged = true;
    boundaries.open = await budget();
    phase = 'wake';
    run(
      world,
      world.binaries.colab,
      ['page', 'write', created.pageId, '--file', '-', '--json'],
      '<main><h1>Native owner second version</h1></main>',
    );
    await expect(page.frameLocator('iframe').locator('h1')).toHaveText(
      'Native owner second version',
      { timeout: 60000 },
    );
    wakeConverged = true;
    boundaries.wake = await budget();
    expect(pageGets).toBe(1);
    expect(queries.some((query) => query.collection === 'authority' && query.documents > 0)).toBe(
      true,
    );
    for (const collection of ['log', 'checkpoints'])
      expect(queries.some((query) => query.collection === collection)).toBe(true);
    expect(writes).toHaveLength(1);
    const mutation = JSON.parse(writes[0]).writes;
    expect(mutation).toHaveLength(1);
    expect(mutation[0].currentDocument).toEqual({ exists: false });
    expect(Object.keys(mutation[0].update.fields).sort()).toEqual([
      'link',
      'page',
      'role',
      'seq',
      'space',
      'token',
      'uid',
    ]);
    expect(mutation[0].update.fields.seq).toEqual({ integerValue: '1' });
    expect(mutation[0].update.name).toContain('/members/');
    phase = 'edit';
    boundaries.beforeEdit = await budget();
    await page.getByTestId('source-toggle').click();
    const source = page.locator('#source-edit');
    await expect(source).toBeVisible();
    await source.fill('<main><h1>Hosted editor requested version</h1></main>');
    await page.getByRole('button', { name: text.save, exact: true }).click();
    await expect(page.getByText('Sent', { exact: true })).toBeVisible({ timeout: 30000 });
    expect(writes).toHaveLength(2);
    const ownMutation = JSON.parse(writes[1]).writes;
    expect(ownMutation).toHaveLength(1);
    expect(ownMutation[0].currentDocument).toEqual({ exists: false });
    expect(ownMutation[0].update.name).toContain('/log/');
    const ownFields = ownMutation[0].update.fields;
    expect(Object.keys(ownFields).sort()).toEqual([
      'envelope',
      'epoch',
      'expiresAt',
      'hash',
      'page',
      'seq',
      'space',
      'stream',
      'uid',
    ]);
    const ownOriginal = Envelope.fromJson(new TextEncoder().encode(ownFields.envelope.stringValue));
    expect(decodeHeader(ownOriginal.header()).context.namespace).toBe('own');
    const originalId = ownMutation[0].update.name.split('/').at(-1);
    const readbacks = gets.filter(
      (get) => get.phase === 'edit' && get.collection === 'log' && get.target === originalId,
    );
    expect(readbacks).toHaveLength(1);
    expect(readbacks[0].status).toBe(200);
    boundaries.requestSent = await budget();
    // Sent is not an optimistic content save: only the owner's new sealed content can render it.
    await expect(page.frameLocator('iframe').locator('h1')).toHaveText(
      'Hosted editor requested version',
      { timeout: 60000 },
    );
    editConverged = true;
    boundaries.edit = await budget();
    expect(
      gets.filter(
        (get) => get.phase === 'edit' && get.collection === 'log' && get.target === originalId,
      ),
    ).toHaveLength(1);
    expect(holderToken.length).toBeGreaterThan(0);
    await page.screenshot({ path: test.info().outputPath('hosted-live.png'), fullPage: true });
    // A holder gets neither enrollment secrets nor the owner's wrap, and cannot escalate itself.
    const deniedGet = async (collection: string, id: string) => {
      const response = await fetch(
        `http://${firestore}/v1/projects/${project}/databases/(default)/documents/x/colab/${collection}/${id}`,
        { headers: { Authorization: `Bearer ${holderToken}` }, signal: AbortSignal.timeout(5000) },
      );
      expect(response.status).toBe(403);
    };
    await deniedGet('links', `${space}_${created.pageId}_${added.linkId}`);
    const other = (await admin('authority')).documents?.find(
      (d) =>
        d.name.includes(created.pageId) &&
        d.fields.kind?.stringValue === 'wrap' &&
        d.fields.recipient?.stringValue !== `link:${added.linkId}`,
    );
    expect(other).toBeDefined();
    await deniedGet('authority', other!.name.split('/').at(-1)!);
    const escalation = await fetch(
      `http://${firestore}/v1/projects/${project}/databases/(default)/documents:commit`,
      {
        method: 'POST',
        headers: { Authorization: `Bearer ${holderToken}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({
          writes: [
            {
              update: {
                ...mutation[0].update,
                fields: { ...mutation[0].update.fields, role: { stringValue: 'bridge' } },
              },
            },
          ],
        }),
        signal: AbortSignal.timeout(5000),
      },
    );
    expect(escalation.status).toBe(403);
    expect(writes).toHaveLength(2);
    expect(paths.some((path) => path.includes('/api/serve-release'))).toBe(false);
    expect(unexpected).toEqual([]);
    expect(crashes).toEqual([]);
    expect(
      await page.evaluate(() => (window as unknown as { __violations: string[] }).__violations),
    ).toEqual([]);
  });
});
