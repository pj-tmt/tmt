import { chromium, expect, test, type ConsoleMessage } from '@playwright/test';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import type { AddressInfo } from 'node:net';
import path from 'node:path';
import { run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';
import type { AcceptanceWorld } from './harness/world.js';

test.afterEach(disposeActiveWorlds);

// #2487: the hosted bundle embedded in the real tmt-colab executable, composed by Remote's own
// offline composer (`compose_hosting`, #3048) and served from a local static server that applies
// the composed per-file headers and rewrites. No Hosting emulator, no network and no host browser.
// The browser uses the canonical demo origin, routed only to these local bytes.
const HOSTED_ORIGIN = 'https://demo-tmt-hosted.web.app';
const INIT_PATH = '/__/firebase/init.json';
// Files the composer adds next to Colab's manifest; Colab never declares anything under it.
const COMPOSER_OWNED_PREFIX = '/tmt/declarations/';
const SHELL_ROUTES = ['/', '/colab', '/p/AbCd1234', '/read/AbCd1234', '/pair'];

interface DeclaredFile {
  path: string;
  sha256: string;
  length: number;
  contentType: string;
  csp?: string;
}
interface Composition {
  version: number;
  files: Array<{ path: string; rawDigest: string; rawLength: number; contentType: string }>;
  config: {
    headers: Array<{ glob?: string; regex?: string; headers: Record<string, string> }>;
    rewrites: Array<{ regex: string; path: string }>;
  };
  digest: string;
}

const sha256 = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');

/** The two public replies Remote reads before any provider call, then Remote's own composition. */
function composed(world: AcceptanceWorld) {
  const declaration = run(world, world.binaries.colab, ['deploy-declaration', '--json']);
  const bundle = run(world, world.binaries.colab, ['hosting-bundle', '--json']);
  const example = path.join(path.dirname(world.binaries.colab), 'examples', 'compose_hosting');
  if (!fs.existsSync(example))
    throw new Error(
      `Missing ${example}. Build it in its own invocation, after the three executables: cd rust && cargo build --locked -p tmt-remote --example compose_hosting. Never add it to the executables' build: an example selected next to tmt-cli turns on its dev-dependency features and the resulting tmt refuses to run in a world (or set TMT_ACCEPTANCE_BIN_DIR to a directory that holds examples/compose_hosting).`,
    );
  const declarationFile = path.join(world.root, 'declaration-envelope.json');
  const bundleFile = path.join(world.root, 'hosting-bundle-reply.json');
  fs.writeFileSync(declarationFile, declaration);
  fs.writeFileSync(bundleFile, bundle);
  const view = JSON.parse(run(world, example, [declarationFile, bundleFile])) as Composition;
  const manifest = (JSON.parse(JSON.parse(declaration).declaration) as { hosting: unknown })
    .hosting as { version: number; files: DeclaredFile[] };
  const files = new Map<string, Buffer>();
  for (const file of JSON.parse(bundle).files as Array<{ path: string; bytesBase64: string }>)
    files.set(file.path, Buffer.from(file.bytesBase64, 'base64'));
  // Remote composes this exact raw declaration alongside the extension's declared files.
  const rawDeclaration = Buffer.from(JSON.parse(declaration).declaration, 'utf8');
  const declarationPath = '/tmt/declarations/colab.json';
  const declared = view.files.find((file) => file.path === declarationPath)!;
  expect(declared.rawDigest).toBe(sha256(rawDeclaration));
  expect(declared.rawLength).toBe(rawDeclaration.byteLength);
  files.set(declarationPath, rawDeclaration);
  return { view, manifest, files };
}

interface Served {
  origin: string;
  /** Every path the server answered, in order; the server is the only network a page has. */
  requests: string[];
}

/** Serve the declared bytes with the composed headers; the shell for the composed rewrites. */
async function serve(
  world: AcceptanceWorld,
  view: Composition,
  files: Map<string, Buffer>,
): Promise<Served> {
  const requests: string[] = [];
  const server = http.createServer((request, response) => {
    const pathname = new URL(request.url ?? '/', 'http://127.0.0.1').pathname;
    requests.push(pathname);
    const rewrite = view.config.rewrites.find((entry) => new RegExp(entry.regex).test(pathname));
    const target = pathname === '/' ? '/index.html' : (rewrite?.path ?? pathname);
    const bytes = files.get(target);
    if (bytes === undefined) {
      response.writeHead(404, { 'Content-Type': 'text/plain; charset=utf-8' });
      response.end('not found');
      return;
    }
    const headers: Record<string, string> = {};
    for (const entry of view.config.headers)
      if (entry.glob === pathname || (entry.regex && new RegExp(entry.regex).test(pathname)))
        Object.assign(headers, entry.headers);
    response.writeHead(200, headers);
    response.end(bytes);
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  world.onDispose(async (stage) => {
    stage('close the hosted bundle server');
    await new Promise<void>((resolve) => {
      server.close(() => resolve());
      server.closeAllConnections();
    });
  });
  return { origin: `http://127.0.0.1:${(server.address() as AddressInfo).port}`, requests };
}

test("the embedded hosted bundle serves its declared bytes and shell under Remote's composition", async () => {
  await withWorld(async (world) => {
    const { view, manifest, files } = composed(world);
    const { origin, requests } = await serve(world, view, files);

    // Remote's composition describes exactly what Colab declared. Remote owns the files it adds
    // under /tmt/declarations/ (the deployed declaration, #3058); they are outside Colab's manifest.
    const colabFiles = view.files.filter((file) => !file.path.startsWith(COMPOSER_OWNED_PREFIX));
    expect(colabFiles.map((file) => file.path)).toEqual(manifest.files.map((file) => file.path));
    for (const [index, declared] of manifest.files.entries()) {
      expect(colabFiles[index].rawDigest).toBe(declared.sha256);
      expect(colabFiles[index].rawLength).toBe(declared.length);
      expect(colabFiles[index].contentType).toBe(declared.contentType);
    }

    // Every declared path answers 200 with its declared bytes, content type and exact CSP.
    const shell = manifest.files.find((file) => file.path === '/index.html')!;
    for (const declared of manifest.files) {
      const response = await fetch(`${origin}${declared.path}`);
      expect(response.status, declared.path).toBe(200);
      const body = new Uint8Array(await response.arrayBuffer());
      expect(body.byteLength, declared.path).toBe(declared.length);
      expect(sha256(body), declared.path).toBe(declared.sha256);
      expect(response.headers.get('content-type'), declared.path).toMatch(
        new RegExp(`^${declared.contentType.replace(/[.*+?^${}()|[\]\\/]/g, '\\$&')}(;|$)`),
      );
      expect(response.headers.get('content-security-policy'), declared.path).toBe(
        declared.csp ?? null,
      );
      expect(response.headers.get('x-content-type-options'), declared.path).toBe('nosniff');
    }

    // The short routes rewrite to the shell with the shell's exact bytes and CSP.
    for (const route of SHELL_ROUTES) {
      const response = await fetch(`${origin}${route}`);
      expect(response.status, route).toBe(200);
      expect(sha256(new Uint8Array(await response.arrayBuffer())), route).toBe(shell.sha256);
      expect(response.headers.get('content-security-policy'), route).toBe(shell.csp ?? null);
    }
    expect((await fetch(`${origin}/not-a-route`)).status).toBe(404);

    // A fresh load of `/` renders the shell: no CSP violation and no request outside the manifest.
    requests.length = 0;
    const browser = await chromium.launch({ headless: true });
    world.onDispose(async (stage) => {
      stage('close the hosted bundle browser');
      await browser.close();
    });
    const context = await browser.newContext({ serviceWorkers: 'block' });
    const outside: string[] = [];
    await context.route('**/*', async (route) => {
      const url = new URL(route.request().url());
      if (url.origin !== HOSTED_ORIGIN) {
        outside.push(url.origin);
        await route.abort();
      } else if (url.pathname === INIT_PATH) {
        await route.fulfill({
          json: {
            apiKey: 'offline-public-key',
            appId: 'offline-public-app',
            authDomain: 'demo-tmt-hosted.firebaseapp.com',
            projectId: 'demo-tmt-hosted',
          },
        });
      } else {
        // Preserve every composed header, including CSP. No browser security override or provider
        // endpoint is used: only the logical origin changes from the local static server.
        const response = await route.fetch({ url: `${origin}${url.pathname}${url.search}` });
        await route.fulfill({ response });
      }
    });
    await context.addInitScript(() => {
      const violations: string[] = [];
      (window as unknown as { __violations: string[] }).__violations = violations;
      document.addEventListener('securitypolicyviolation', (event) =>
        violations.push(`${event.violatedDirective} ${event.blockedURI}`),
      );
    });
    const page = await context.newPage();
    const errors: ConsoleMessage[] = [];
    const crashes: string[] = [];
    const statuses = new Map<string, number>();
    page.on('console', (message) => {
      if (message.type() === 'error') errors.push(message);
    });
    page.on('pageerror', (error) => crashes.push(error.message));
    page.on('response', (response) => {
      if (new URL(response.url()).origin === HOSTED_ORIGIN)
        statuses.set(new URL(response.url()).pathname, response.status());
    });
    await page.goto(`${HOSTED_ORIGIN}/`, { waitUntil: 'networkidle' });
    await expect(
      page.getByRole('button', { name: 'Sign in to Firestore', exact: true }),
    ).toBeVisible();
    await expect(page.getByText('Open a share link from Colab to read a page here.')).toBeVisible();
    await expect(
      page.getByText('This preview uses local sample pages.', { exact: false }),
    ).toHaveCount(0);
    expect(statuses.has('/api/serve-release')).toBe(false);
    await page.screenshot({ path: test.info().outputPath('hosted-sign-in.png'), fullPage: true });
    await test.info().attach('hosted-sign-in', {
      path: test.info().outputPath('hosted-sign-in.png'),
      contentType: 'image/png',
    });

    expect(outside).toEqual([]);
    expect(crashes).toEqual([]);
    expect(await page.evaluate(() => (window as any).__violations)).toEqual([]);
    const declaredPaths = new Set(manifest.files.map((file) => file.path));
    const unexpected = [...new Set(requests)].filter(
      (pathname) => pathname !== '/' && !declaredPaths.has(pathname) && !files.has(pathname),
    );
    expect(unexpected).toEqual([]);
    // Every shell reference and bootstrap reply is served; there is no native-door allowance.
    const html = Buffer.from(files.get('/index.html')!).toString('utf8');
    const referenced = [...html.matchAll(/(?:src|href)="(\/assets\/[^"]+)"/g)].map((m) => m[1]);
    expect(referenced.length).toBeGreaterThan(0);
    for (const asset of referenced) expect(statuses.get(asset), asset).toBe(200);
    for (const [pathname, status] of statuses) expect(status, pathname).toBe(200);
    expect(statuses.get(INIT_PATH)).toBe(200);
    expect(statuses.get('/tmt/declarations/colab.json')).toBe(200);
    expect(errors.map((message) => message.text())).toEqual([]);
  });
});
