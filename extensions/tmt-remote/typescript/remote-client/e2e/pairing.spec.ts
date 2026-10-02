import { expect, test, chromium, type Browser } from '@playwright/test';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
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
let origin: string, env: NodeJS.ProcessEnv, pair: ChildProcessWithoutNullStreams | undefined;
test.beforeEach(async () => {
  // Short root: Unix socket paths are limited to about 100 bytes.
  root = await mkdtemp('/tmp/tmt-e2e-');
  await writeFile(
    join(root, 'core'),
    `#!/bin/sh\ninput=$(cat)\ncase "$input" in *storage.root*) printf '{"dataRoot":"${root}/state"}';; *) printf '{"version":1,"limits":{"inputBytes":1024,"outputBytes":4096}}';; esac\n`,
    { mode: 0o700 },
  );
  env = { PATH: process.env.PATH, HOME: root, TMT_EXECUTABLE: join(root, 'core') };
  serve = spawn(BINARY, ['serve', '--json'], { env });
  const descriptor = await lines(serve).next();
  origin = new URL(descriptor.address as string).origin;
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

  const cookies = await context.cookies(`${origin}/x/`);
  expect(cookies).toHaveLength(1);
  expect(cookies[0]).toMatchObject({
    name: 'tmt_door',
    path: '/x/',
    httpOnly: true,
    sameSite: 'Strict',
  });
  const app = await context.newPage();
  await app.goto(`${origin}/x/colab/home`);
  const device = JSON.parse((await app.locator('#context').textContent())!) as Record<
    string,
    unknown
  >;
  expect(device).toMatchObject({ kind: 'browser', origin, name: 'E2E browser', owner: true });

  // The page-facing SDK exposes no caller-chosen extension and keeps the key opaque.
  const result = await app.evaluate(async () => {
    const sdk = (await import('/sdk/remote-v1.js' as string)) as {
      certifyKey(purpose: 'sign', key: Uint8Array): Promise<Record<string, unknown>>;
    };
    const first = await sdk.certifyKey('sign', new Uint8Array(32).fill(7));
    const again = await sdk.certifyKey('sign', new Uint8Array(32).fill(7));
    const stored = await new Promise<{ handle: CryptoKey }>((resolve) => {
      const open = indexedDB.open('tmt-remote', 1);
      open.onsuccess = () => {
        const get = open.result.transaction('device').objectStore('device').get('device');
        get.onsuccess = () => resolve(get.result as { handle: CryptoKey });
      };
    });
    return {
      exports: Object.keys(sdk).sort(),
      first,
      same: first.issuedAtMs === again.issuedAtMs,
      extractable: stored.handle.extractable,
    };
  });
  expect(result.exports).toEqual(['certifyKey', 'reopenSession']);
  expect(result.first).toMatchObject({ extension: 'colab', purpose: 'sign' });
  expect(result.same).toBe(true);
  expect(result.extractable).toBe(false);

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
  await app.reload();
  expect(JSON.parse((await app.locator('#context').textContent())!)).toMatchObject({
    deviceId: device.deviceId,
  });
  await exited(pair);
});
