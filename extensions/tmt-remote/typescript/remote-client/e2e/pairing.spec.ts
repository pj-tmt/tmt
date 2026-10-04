import { expect, test, chromium, type Browser } from '@playwright/test';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { createPublicKey, verify } from 'node:crypto';
import { extCertSigningBytes } from '../src/canonical-bytes.js';
import type { ExtCertificate } from '../src/device.js';
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer, type Server } from 'node:net';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

/**
 * Chromium smoke over a real `tmt remote serve`: the owner runs `pair --json`,
 * the browser opens the link, both sides show the same four words, the owner
 * confirms, and the door cookie then carries the device context to a fixture
 * colab served on its owner-only socket. Build the binary first with
 * `cargo build -p tmt-remote`, or point TMT_REMOTE_BINARY at one.
 */
const BINARY =
  process.env.TMT_REMOTE_BINARY ??
  fileURLToPath(new URL('../../../../../rust/target/debug/tmt-remote', import.meta.url));

type Lines = { next(): Promise<Record<string, unknown>> };
function lines(child: ChildProcessWithoutNullStreams): Lines {
  const queue: string[] = [];
  const waiting: ((line: string) => void)[] = [];
  createInterface({ input: child.stdout }).on('line', (line) => {
    const resolve = waiting.shift();
    if (resolve) resolve(line);
    else queue.push(line);
  });
  return {
    async next() {
      const line =
        queue.shift() ??
        (await new Promise<string>((resolve, reject) => {
          const timer = setTimeout(() => reject(new Error('No line within 30 s.')), 30000);
          waiting.push((value) => {
            clearTimeout(timer);
            resolve(value);
          });
        }));
      return JSON.parse(line) as Record<string, unknown>;
    },
  };
}
/** Fixture colab: a page that shows the forwarded device context and may load the SDK. */
async function colab(socket: string): Promise<Server> {
  const server = createServer((connection) => {
    let head = '';
    connection.on('data', (chunk) => {
      head += chunk.toString('latin1');
      if (!head.includes('\r\n\r\n')) return;
      const context = /^tmt-device-context: (.*)$/im.exec(head)?.[1] ?? 'none';
      const body = `<!doctype html><title>colab</title><pre id="context">${context
        .replaceAll('&', '&amp;')
        .replaceAll('<', '&lt;')}</pre>`;
      connection.end(
        `HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-security-policy: default-src 'none'; script-src 'self'; connect-src 'self'\r\ncontent-length: ${Buffer.byteLength(body)}\r\n\r\n${body}`,
      );
    });
  });
  await new Promise<void>((resolve) => server.listen(socket, resolve));
  await chmod(socket, 0o600);
  return server;
}

/** Resolves once the child has exited, including before this call. */
function exited(child: ChildProcessWithoutNullStreams): Promise<unknown> {
  return child.exitCode !== null || child.signalCode !== null
    ? Promise.resolve()
    : new Promise((resolve) => child.once('exit', resolve));
}

let root: string, serve: ChildProcessWithoutNullStreams, browser: Browser, fixture: Server;
let origin: string,
  mounts: string,
  env: NodeJS.ProcessEnv,
  pair: ChildProcessWithoutNullStreams | undefined;
test.beforeEach(async () => {
  // Short root: Unix socket paths are limited to about 100 bytes.
  root = await mkdtemp('/tmp/tmt-e2e-');
  await writeFile(
    join(root, 'core.mjs'),
    await readFile(new URL('./core-fixture.mjs', import.meta.url)),
    { mode: 0o700 },
  );
  env = {
    PATH: process.env.PATH,
    HOME: root,
    TMT_EXECUTABLE: join(root, 'core.mjs'),
    TMT_FIXTURE_ROOT: root,
  };
  serve = spawn(BINARY, ['serve', '--json'], { env });
  const descriptor = await lines(serve).next();
  origin = new URL(descriptor.address as string).origin;
  // Mounts live under the machine's unpredictable route prefix.
  mounts = `${descriptor.address as string}/x/`;
  await mkdir(join(root, 'state/colab'), { recursive: true, mode: 0o700 });
  fixture = await colab(join(root, 'state/colab/door.sock'));
  browser = await chromium.launch();
});
test.afterEach(async () => {
  await browser.close();
  // A failed run can leave the owner's pair client waiting; end it first.
  pair?.kill('SIGTERM');
  if (pair) await exited(pair);
  pair = undefined;
  serve.kill('SIGTERM');
  await exited(serve);
  await new Promise((resolve) => fixture.close(resolve));
  await rm(root, { recursive: true, force: true });
});

test('a browser pairs, gets a door session and certifies only its own extension', async () => {
  pair = spawn(BINARY, ['pair', '--json'], { env });
  const events = lines(pair);
  const offer = await events.next();
  const link = offer.link as string;
  const code = link.split('#')[1]!;
  const context = await browser.newContext();
  const requested: string[] = [];
  context.on('request', (request) => requested.push(request.url()));
  const page = await context.newPage();
  await page.goto(link);
  // The page removed the fragment before anything else ran.
  await page.waitForFunction(() => location.hash === '');
  await page.fill('#name', 'E2E browser');
  await page.click('button');
  await expect(page.locator('#words')).toBeVisible();
  const candidate = await events.next();
  expect(candidate.event).toBe('candidate');
  await expect(page.locator('#words')).toHaveText(
    `Words: ${(candidate.words as string[]).join(' ')}`,
  );
  pair.stdin.write('confirm\n');
  expect((await events.next()).reason).toBe('paired');
  await expect(page.locator('#status')).toHaveText(
    'This browser is paired. You can close this page.',
  );
  expect(requested.some((url) => url.includes(code))).toBe(false);

  const cookies = await context.cookies(mounts);
  expect(cookies).toHaveLength(1);
  expect(cookies[0]).toMatchObject({
    name: 'tmt_door',
    path: new URL(mounts).pathname,
    httpOnly: true,
    sameSite: 'Strict',
  });
  const app = await context.newPage();
  const dialogs: string[] = [];
  app.on('dialog', (dialog) => {
    dialogs.push(dialog.type());
    void dialog.dismiss();
  });
  // The old root mount space is gone.
  expect((await app.goto(`${origin}/x/colab/home`))?.status()).toBe(404);
  await app.goto(`${mounts}colab/home`);
  const device = JSON.parse((await app.locator('#context').textContent())!) as Record<
    string,
    unknown
  >;
  expect(device).toMatchObject({ kind: 'browser', origin, name: 'E2E browser', owner: true });

  // The page-facing SDK exposes no caller-chosen extension and keeps the key opaque.
  const result = await app.evaluate(async () => {
    const sdk = (await import('/sdk/remote-v1.js' as string)) as {
      certifyKey(
        purpose: 'sign' | 'enc',
        key: Uint8Array,
      ): Promise<import('../src/device.js').ExtCertificate>;
    };
    const database = await new Promise<IDBDatabase>((resolve) => {
      const open = indexedDB.open('tmt-remote', 1);
      open.onsuccess = () => resolve(open.result);
    });
    const stored = await new Promise<{ handle: CryptoKey; certificates?: unknown }>((resolve) => {
      const get = database.transaction('device').objectStore('device').get('device');
      get.onsuccess = () => resolve(get.result as { handle: CryptoKey; certificates?: unknown });
    });
    const newRecordHasCache = Object.hasOwn(stored, 'certificates');
    const key = new Uint8Array(32).fill(7);
    const first = await sdk.certifyKey('sign', key);
    const enc = await sdk.certifyKey('enc', new Uint8Array(32).fill(8));
    // Restore the old record shape with a stale certificate for exactly this key.
    const legacy = { ...stored, certificates: [{ ...first, issuedAtMs: 1 }] };
    await new Promise<void>((resolve, reject) => {
      const transaction = database.transaction('device', 'readwrite');
      transaction.objectStore('device').put(legacy, 'device');
      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(transaction.error);
    });
    const now = Date.now;
    const nextTime = first.issuedAtMs + 60_000;
    let again;
    try {
      // Deterministic clock advancement detects stale reuse without waiting.
      Date.now = () => nextTime;
      again = await sdk.certifyKey('sign', key);
    } finally {
      Date.now = now;
    }
    // Extra JavaScript arguments cannot select another extension.
    const override = await Reflect.apply(sdk.certifyKey, undefined, ['sign', key, 'other']);
    database.close();
    return {
      exports: Object.keys(sdk).sort(),
      first,
      enc,
      again,
      override,
      nextTime,
      newRecordHasCache,
      extractable: stored.handle.extractable,
    };
  });
  expect(result.exports).toEqual([
    'ClientError',
    'RefusalError',
    'certifyKey',
    'operations',
    'reopenSession',
  ]);
  expect(result.newRecordHasCache).toBe(false);
  expect(result.again.issuedAtMs).toBe(result.nextTime);
  expect(result.again.issuedAtMs).toBeGreaterThanOrEqual(result.first.issuedAtMs);
  expect(result.again.signature).not.toBe(result.first.signature);
  expect(result.extractable).toBe(false);
  const deviceKey = createPublicKey({
    key: Buffer.concat([
      Buffer.from('302a300506032b6570032100', 'hex'),
      Buffer.from(device.publicKey as string, 'base64url'),
    ]),
    format: 'der',
    type: 'spki',
  });
  expect(result.first).toMatchObject({ purpose: 'sign' });
  expect(result.enc).toMatchObject({ purpose: 'enc' });
  for (const certificate of [
    result.first,
    result.enc,
    result.again,
    result.override,
  ] as ExtCertificate[]) {
    expect(certificate.extension).toBe('colab');
    expect(
      verify(
        null,
        extCertSigningBytes({
          ...certificate,
          publicKey: Buffer.from(certificate.publicKey, 'base64url'),
        }),
        deviceKey,
        Buffer.from(certificate.signature, 'base64url'),
      ),
    ).toBe(true);
  }

  // Without its cookie the page is non-owner until it silently reopens a session.
  await context.clearCookies();
  await app.reload();
  await expect(app.locator('#context')).toHaveText('none');
  await app.evaluate(async () => {
    const sdk = (await import('/sdk/remote-v1.js' as string)) as {
      reopenSession(): Promise<unknown>;
    };
    await sdk.reopenSession();
  });
  const asked = await app.evaluate(async () => {
    const sdk = (await import('/sdk/remote-v1.js' as string)) as typeof import('../src/browser.js');
    const remote = sdk.operations(await sdk.reopenSession());
    const agents = await remote.listAgents();
    if (!agents[0]) throw new Error('No permitted agent.');
    const input = {
      operationId: crypto.randomUUID(),
      agentId: agents[0].id,
      message: 'Browser direct ask! e\u0301 🎯',
    };
    const sent = await remote.send(input);
    if (sent.state !== 'accepted') throw new Error(`Send was ${sent.state}`);
    const observed = await remote.operation(input.operationId);
    const result = await remote.result(sent.requestId);
    return { agents, input, sent, observed, result };
  });
  expect(asked.agents).toEqual([
    { id: '00000000-0000-0000-0000-000000000001', name: 'Browser agent', presence: 'active' },
  ]);
  expect(asked.sent).toMatchObject({ state: 'accepted', operationId: asked.input.operationId });
  expect(asked.observed).toEqual(asked.sent);
  expect(asked.result).toEqual({
    state: 'replied',
    requestId: asked.sent.requestId,
    message: 'Browser retained final 🎯',
  });
  const coreCalls = (await readFile(join(root, 'core-calls.jsonl'), 'utf8'))
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line) as { operation: string });
  expect(coreCalls.filter((call) => call.operation === 'dispatch.create')).toHaveLength(1);
  expect(coreCalls.filter((call) => call.operation === 'requests.show')).toHaveLength(1);
  expect(await readFile(join(root, 'dispatched-message.txt'), 'utf8')).toBe(
    `[remote: E2E browser]\n${asked.input.message}`,
  );
  await app.reload();
  expect(JSON.parse((await app.locator('#context').textContent())!)).toMatchObject({
    deviceId: device.deviceId,
  });
  await exited(pair);
  const revoke = spawn(BINARY, ['devices', 'revoke', device.deviceId as string, '--json'], { env });
  const revoked = await lines(revoke).next();
  await exited(revoke);
  expect(revoke.exitCode).toBe(0);
  expect(revoked.device).toMatchObject({ clientId: device.deviceId, revoked: true, revision: 2 });
  // Keep the revoked cookie and certificate: neither is fresh owner authority.
  await app.reload();
  await expect(app.locator('#context')).toHaveText('none');
  const refused = await app.evaluate(async () => {
    const sdk = (await import('/sdk/remote-v1.js' as string)) as {
      reopenSession(): Promise<unknown>;
    };
    try {
      await sdk.reopenSession();
      return 'opened';
    } catch (error) {
      return error instanceof Error ? error.message : String(error);
    }
  });
  expect(refused).toBe('The session was refused.');
  await app.reload();
  await expect(app.locator('#context')).toHaveText('none');
  // Revocation changes grant admission, not the mathematics of a retained signature.
  for (const certificate of [result.first, result.enc]) {
    expect(
      verify(
        null,
        extCertSigningBytes({
          ...certificate,
          publicKey: Buffer.from(certificate.publicKey, 'base64url'),
        }),
        deviceKey,
        Buffer.from(certificate.signature, 'base64url'),
      ),
    ).toBe(true);
  }
  expect(dialogs).toEqual([]);
});

// Optional exported evidence uses the same real-door fixture as the pairing smoke.
// No mocked HTML or network responses enter the captures.
test('browser pages use local tokens in both schemes and fit desktop and mobile', async () => {
  const captures = process.env.TMT_REMOTE_CAPTURE_DIR;
  if (captures) await mkdir(captures, { recursive: true });
  for (const colorScheme of ['light', 'dark'] as const) {
    for (const width of [1440, 390]) {
      const context = await browser.newContext({
        colorScheme,
        viewport: { width, height: 900 },
      });
      try {
        const requests: string[] = [];
        const violations: string[] = [];
        context.on('request', (request) => requests.push(request.url()));
        const page = await context.newPage();
        await page.addInitScript(() => {
          document.addEventListener('securitypolicyviolation', (event) => {
            console.error(`CSP violation: ${event.violatedDirective}`);
          });
        });
        page.on('console', (message) => {
          if (message.text().startsWith('CSP violation:')) violations.push(message.text());
        });
        const inspect = async (state: string) => {
          const look = await page.evaluate(() => {
            const sheet = document.querySelector('.sheet')!;
            const style = getComputedStyle(sheet);
            return {
              paper: getComputedStyle(document.body).backgroundColor,
              sheet: style.backgroundColor,
              text: getComputedStyle(document.body).color,
              shadow: style.boxShadow,
              radius: style.borderRadius,
              overflow: document.documentElement.scrollWidth > innerWidth,
              // Playwright restores the input caret with an empty style attribute.
              inline: document.querySelectorAll('[style]:not([style=""]), style, script:not([src])')
                .length,
            };
          });
          expect(look).toMatchObject({
            paper: colorScheme === 'light' ? 'rgb(244, 246, 251)' : 'rgb(26, 27, 38)',
            sheet: colorScheme === 'light' ? 'rgb(255, 255, 255)' : 'rgb(22, 22, 30)',
            text: colorScheme === 'light' ? 'rgb(52, 59, 88)' : 'rgb(192, 202, 245)',
            radius: '0px',
            overflow: false,
            inline: 0,
          });
          expect(look.shadow).toContain('6px 6px 0px 0px');
          if (captures) {
            await page.screenshot({
              path: join(captures, `${state}-${width}-${colorScheme}.png`),
              fullPage: true,
            });
          }
        };
        expect((await page.goto(origin))?.status()).toBe(200);
        await inspect('landing');
        expect((await page.goto(`${origin}/pair/`))?.status()).toBe(404);
        await expect(page.getByRole('heading')).toHaveText('This page is unavailable');
        await inspect('error');

        pair = spawn(BINARY, ['pair', '--json'], { env });
        const events = lines(pair);
        const offer = await events.next();
        await page.goto(offer.link as string);
        await expect(page.locator('#pair')).toBeVisible();
        await inspect('pairing');
        // Keyboard-only submission exercises the visible focus and form behavior.
        await page.locator('#name').focus();
        await page.keyboard.press('Tab');
        await expect(page.getByRole('button', { name: 'Pair', exact: true })).toBeFocused();
        await page.keyboard.press('Enter');
        const candidate = await events.next();
        expect(candidate.event).toBe('candidate');
        await expect(page.locator('#words')).toHaveText(
          `Words: ${(candidate.words as string[]).join(' ')}`,
        );
        await expect(page.locator('#status')).toHaveAttribute('data-state', 'waiting');
        await inspect('confirmation');
        pair.stdin.write('refuse\n');
        expect((await events.next()).reason).toBe('refused');
        await exited(pair);
        pair = undefined;
        await expect(page.locator('#status')).toHaveAttribute('data-state', 'blocked');
        await expect(page.locator('#status')).toHaveText(
          'Pairing did not complete. Run tmt remote pair again for a new link.',
        );
        await page.goto(`${origin}/pair/abc`);
        await expect(page.locator('#status')).toHaveAttribute('data-state', 'blocked');
        await expect(page.locator('#pair')).toBeHidden();
        expect(requests.every((url) => new URL(url).origin === origin)).toBe(true);
        expect(violations).toEqual([]);
      } finally {
        await context.close();
      }
    }
  }
  const coreCalls = (await readFile(join(root, 'core-calls.jsonl'), 'utf8'))
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line) as { operation: string });
  // Serve discovers capabilities/root once; each owner pair client discovers its root.
  expect(coreCalls.map((call) => call.operation)).toEqual([
    'capabilities',
    'storage.root',
    ...Array<string>(4).fill('storage.root'),
  ]);
});
