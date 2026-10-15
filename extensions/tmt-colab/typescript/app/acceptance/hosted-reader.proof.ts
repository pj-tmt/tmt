import { chromium, expect, test } from '@playwright/test';
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
test('a hosted link holder verifies native authority and admits empty discovery without a local door', async () => {
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
    const added = JSON.parse(
      run(world, world.binaries.colab, ['share', 'link', 'add', created.pageId, '--yes', '--json']),
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
    const queries: Array<{ collection: string; documents: number }> = [];
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
        const response = await route.fetch({
          url: `http://${firestore}${url.pathname}${url.search}`,
        });
        if (response.ok() && url.pathname.endsWith(':runQuery')) {
          const input = request.postDataJSON() as {
            structuredQuery: { from: Array<{ collectionId: string }> };
          };
          const rows = (await response.json()) as Array<{ document?: unknown }>;
          queries.push({
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
    const crashes: string[] = [];
    page.on('pageerror', (error) => crashes.push(error.message));
    await page.goto(`${origin}/read/${created.pageId}${fragment}`);
    await expect(page).toHaveURL(`${origin}/read/${created.pageId}`);
    await page.getByRole('button', { name: 'Sign in to Firestore', exact: true }).click();
    await expect(page.getByText('Live', { exact: true })).toBeVisible({ timeout: 25000 });
    await expect(page.locator('iframe')).toHaveCount(1);
    // #3127: the native CLI's local stream is currently excluded from publication. Do not seed
    // writer authority or claim content convergence here. Signed fixture tests cover content;
    // this real proof covers bootstrap, authority, and truthful empty finite discovery.
    expect(pageGets).toBe(1);
    expect(queries.some((query) => query.collection === 'authority' && query.documents > 0)).toBe(
      true,
    );
    for (const collection of ['log', 'checkpoints'])
      expect(queries.some((query) => query.collection === collection)).toBe(true);
    await test.info().attach('provider-read-summary', {
      body: JSON.stringify({ pageGets, queries }),
      contentType: 'application/json',
    });
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
                fields: { ...mutation[0].update.fields, role: { stringValue: 'editor' } },
              },
            },
          ],
        }),
        signal: AbortSignal.timeout(5000),
      },
    );
    expect(escalation.status).toBe(403);
    expect(writes).toHaveLength(1);
    expect(paths.some((path) => path.includes('/api/serve-release'))).toBe(false);
    expect(unexpected).toEqual([]);
    expect(crashes).toEqual([]);
    expect(
      await page.evaluate(() => (window as unknown as { __violations: string[] }).__violations),
    ).toEqual([]);
  });
});
