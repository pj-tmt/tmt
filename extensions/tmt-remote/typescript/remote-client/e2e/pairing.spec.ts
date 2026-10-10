import { expect, test, chromium, type Browser, type Page } from '@playwright/test';
import { spawn, execFileSync, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { createHash, createPublicKey, verify } from 'node:crypto';
import { extCertSigningBytes } from '../src/canonical-bytes.js';
import type { ExtCertificate } from '../src/device.js';
import { appendFile, chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createConnection, createServer, type Server, type Socket } from 'node:net';
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
// Expected computed presentation comes from main's current browser roles, not a local palette.
const { header: headerTokens, browser: browserTokens } = JSON.parse(
  await readFile(
    fileURLToPath(new URL('../../../../../design/tokens/tokens.json', import.meta.url)),
    'utf8',
  ),
) as {
  header: Record<string, string>;
  browser: {
    color: Record<string, Record<string, string>>;
    surface: Record<string, Record<string, string>>;
  };
};
function tokenRgb(group: 'color' | 'surface', name: string, theme: 'light' | 'dark'): string {
  const hex = browserTokens[group][name]![theme]!;
  return `rgb(${[1, 3, 5].map((start) => parseInt(hex.slice(start, start + 2), 16)).join(', ')})`;
}
async function expectStateColor(
  page: Page,
  scheme: 'light' | 'dark',
  state: 'waiting' | 'blocked' | 'working',
) {
  expect(await page.locator('.state-mark').evaluate((mark) => getComputedStyle(mark).color)).toBe(
    tokenRgb('color', state, scheme),
  );
  expect(
    await page
      .locator('.sheet')
      .first()
      .evaluate((sheet) => getComputedStyle(sheet).boxShadow),
  ).toBe('none');
}

/** Observe feedback after an action without scrolling to manufacture visibility. */
async function feedbackBounds(page: Page, selector: string) {
  const measurement = await page.locator(selector).evaluateAll((elements) => ({
    scrollY,
    viewportHeight: innerHeight,
    viewportWidth: innerWidth,
    bounds: elements.map((element) => {
      const box = element.getBoundingClientRect();
      return {
        top: box.top,
        bottom: box.bottom,
        left: box.left,
        right: box.right,
        text: element.textContent,
      };
    }),
  }));
  expect(measurement.bounds.length).toBeGreaterThan(0);
  for (const box of measurement.bounds) {
    expect(box.top, JSON.stringify(measurement)).toBeGreaterThanOrEqual(48);
    expect(box.bottom, JSON.stringify(measurement)).toBeLessThanOrEqual(measurement.viewportHeight);
    expect(box.left).toBeGreaterThanOrEqual(0);
    expect(box.right).toBeLessThanOrEqual(measurement.viewportWidth);
  }
  return measurement;
}

/** Capture the real served state; media and viewport changes never activate it. */
async function captureState(
  page: Page,
  state: string,
  lookAt: string,
  widths = [1440, 390, 320],
  themes: readonly ('light' | 'dark')[] = ['light', 'dark'],
  contained?: string,
): Promise<void> {
  const { theme, controls } = await page.evaluate(() => ({
    theme: matchMedia('(prefers-color-scheme: dark)').matches
      ? ('dark' as const)
      : ('light' as const),
    controls: Array.from(
      document.querySelectorAll<HTMLButtonElement>('button.tmt-ui-action:disabled'),
    ).map((button) => {
      const style = getComputedStyle(button);
      const form = button.closest('form');
      const input = form?.querySelector('input');
      const position =
        Array.from(button.parentElement?.children ?? [])
          .filter((child) => child.tagName === 'BUTTON')
          .indexOf(button) + 1;
      const owner = form?.id
        ? `#${CSS.escape(form.id)}`
        : input?.id
          ? `form:has(#${CSS.escape(input.id)})`
          : '';
      return {
        context: {
          selector: button.id
            ? `#${CSS.escape(button.id)}`
            : `${owner} button:nth-of-type(${position})`.trim(),
          label: button.textContent?.trim(),
          disabled: button.matches(':disabled'),
          disabledAttribute: button.getAttribute('disabled'),
          ariaBusy: button.getAttribute('aria-busy'),
          connected: button.isConnected,
          refreshBusy: document.querySelector<HTMLButtonElement>('#refresh')?.disabled ?? null,
        },
        style: {
          color: style.color,
          background: style.backgroundColor,
          opacity: style.opacity,
          shadow: style.boxShadow,
        },
      };
    }),
  }));
  for (const control of controls)
    expect(control.style, `${state}: ${JSON.stringify(control)}`).toEqual({
      color: tokenRgb('color', 'disabled-text', theme),
      background: tokenRgb('surface', 'disabled', theme),
      opacity: '1',
      shadow: 'none',
    });
  if (contained) await feedbackBounds(page, contained);
  const directory = process.env.TMT_REMOTE_CAPTURE_DIR;
  if (!directory) return;
  await mkdir(directory, { recursive: true });
  const viewport = page.viewportSize();
  const position = await page.evaluate(() => ({ x: scrollX, y: scrollY }));
  const originalTheme = await page.evaluate(() =>
    matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
  );
  try {
    for (const theme of themes) {
      await page.emulateMedia({ colorScheme: theme });
      for (const width of widths) {
        await page.setViewportSize({ width, height: 900 });
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
          true,
        );
        const path = join(directory, `${state}-${width}-${theme}.png`);
        let measurement: unknown;
        if (contained) {
          measurement = await feedbackBounds(page, contained);
        } else await page.evaluate(() => scrollTo(0, 0));
        await page.screenshot({ path, fullPage: !contained });
        await appendFile(
          join(directory, 'index.jsonl'),
          JSON.stringify({ state, viewport: `${width}x900`, theme, path, lookAt, measurement }) +
            '\n',
        );
      }
    }
  } finally {
    await page.emulateMedia({ colorScheme: originalTheme });
    if (viewport) await page.setViewportSize(viewport);
    await page.evaluate(({ x, y }) => scrollTo(x, y), position);
  }
}

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
async function colab(socket: string, recordHead: (head: string) => void): Promise<Server> {
  const server = createServer((connection) => {
    let head = '';
    connection.on('data', (chunk) => {
      head += chunk.toString('latin1');
      if (!head.includes('\r\n\r\n')) return;
      connection.removeAllListeners('data');
      recordHead(head);
      expect(head).not.toContain('tmt-session');
      if (/^upgrade: websocket$/im.test(head)) {
        const key = /^sec-websocket-key: (.*)$/im.exec(head)?.[1]?.trim();
        if (!key) throw new Error('No WebSocket key.');
        const accept = createHash('sha1')
          .update(key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11')
          .digest('base64');
        connection.write(
          `HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`,
        );
        connection.on('data', (frame: Buffer) => {
          if ((frame[0]! & 15) === 8) connection.destroy();
        });
        return;
      }
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

// A fragmented request can deliver more data after the fixture ends its one response.
// Drive that ordering at the accepted real socket, without relying on packet coalescing.
test('fixture Colab answers a fragmented HTTP request only once', async () => {
  const accepted = new Promise<Socket>((resolve) => fixture.once('connection', resolve));
  const client = createConnection(join(root, 'state/colab/door.sock'));
  client.resume();
  const connection = await accepted;
  const closed = new Promise<void>((resolve) => connection.once('close', () => resolve()));
  const errors: string[] = [];
  connection.on('error', (error) => errors.push(error.message));
  try {
    connection.emit('data', Buffer.from('POST /fixture HTTP/1.1\r\ncontent-length: 4\r\n'));
    expect(connection.writableEnded).toBe(false);
    connection.emit('data', Buffer.from('\r\n'));
    expect(connection.writableEnded).toBe(true);
    connection.emit('data', Buffer.from('body'));
    await closed;
    expect(errors, 'HTTP body must not start a second fixture response').toEqual([]);
    expect(connection.listenerCount('data')).toBe(0);
  } finally {
    client.destroy();
  }
});

/** Resolves once the child has exited, including before this call. */
function exited(child: ChildProcessWithoutNullStreams): Promise<unknown> {
  return child.exitCode !== null || child.signalCode !== null
    ? Promise.resolve()
    : new Promise((resolve) => child.once('exit', resolve));
}

let publicHeads: string[];
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
  publicHeads = [];
  fixture = await colab(join(root, 'state/colab/door.sock'), (head) => publicHeads.push(head));
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

// The extension is a public-entry stand-in, not Colab app/routing acceptance.
test('short public entries preserve the browser URL and map the SDK mount without authority', async () => {
  const page = await browser.newPage();
  const network: string[] = [];
  const mount = `${new URL(mounts).pathname}colab/`;
  page.on('request', (request) => network.push(request.url()));
  for (const path of ['/colab', '/colab/', '/p/abcd#t=thread-canary', '/read/abcd#read-canary']) {
    const address = `${origin}${path}`;
    const response = await page.goto(address);
    expect(response?.status()).toBe(200);
    expect(response?.request().redirectedFrom()).toBeNull();
    expect(page.url()).toBe(address);
    await expect(page.locator('#context')).toHaveText('none');
    const lookup = await page.evaluate(() =>
      fetch('/sdk/mount', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path: location.pathname }),
      }).then((reply) => reply.json()),
    );
    expect(lookup).toEqual({
      machineId: expect.any(String),
      windowId: expect.any(String),
      address: mounts.slice(0, -3),
      extension: 'colab',
      mount,
    });
    expect((await page.reload())?.status()).toBe(200);
    expect(page.url()).toBe(address);
    await expect(page.locator('#context')).toHaveText('none');
  }
  expect(publicHeads.map((head) => head.split(' ')[1])).toEqual([
    '/',
    '/',
    '/',
    '/',
    '/p/abcd',
    '/p/abcd',
    '/read/abcd',
    '/read/abcd',
  ]);
  for (const head of publicHeads) {
    expect(head).toContain(`tmt-mount: ${mount}\r\n`);
    expect(head).not.toMatch(/^cookie:|^tmt-device-context:|^tmt-origin:|^upgrade:/im);
    expect(head).not.toMatch(/thread-canary|read-canary|#/);
  }
  expect(network.every((url) => !url.includes('#'))).toBe(true);
  // BFCache restoration need not return a new network response.
  await page.goBack();
  expect(page.url()).toBe(`${origin}/p/abcd#t=thread-canary`);
  await expect(page.locator('#context')).toHaveText('none');
  await page.goForward();
  expect(page.url()).toBe(`${origin}/read/abcd#read-canary`);
  await page.close();
});

/** Hold an actual route until the test has exercised the pre-readiness page. */
function routeBarrier() {
  let arrive!: () => void;
  let release!: () => void;
  const reached = new Promise<void>((resolve) => {
    arrive = resolve;
  });
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  return { reached, held, arrive, release };
}

/** Observe native form refusal before any bootstrap/SDK script runs. */
async function watchPairing(page: Page) {
  const posts: string[] = [];
  page.on('request', (request) => {
    if (request.method() === 'POST' && new URL(request.url()).pathname.endsWith('/pair'))
      posts.push(request.postData()!);
  });
  await page.addInitScript(() => {
    const violations: string[] = [];
    Object.assign(window, { pairingViolations: violations });
    document.addEventListener('securitypolicyviolation', (event) => {
      violations.push(event.violatedDirective);
    });
  });
  return {
    posts,
    violations: () =>
      page.evaluate(() => (window as unknown as { pairingViolations: string[] }).pairingViolations),
  };
}

/** Raw click deliberately avoids Playwright's enabled-control auto-wait. */
async function earlyPairingAttempt(page: Page) {
  await page.locator('#name').fill('Readiness browser');
  const box = await page.getByRole('button', { name: 'Pair', exact: true }).boundingBox();
  if (!box) throw new Error('No pairing control.');
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  // Native submission in the broken page can leave navigation pending; do not auto-wait it.
  await page.evaluate(() => document.getElementById('name')!.focus());
  await page.keyboard.press('Enter');
}

for (const phase of ['SDK import', 'offer response']) {
  test(`pairing stays unavailable during ${phase} and preserves the deliberate attempt`, async () => {
    pair = spawn(BINARY, ['pair', '--json'], { env });
    const events = lines(pair);
    const offer = await events.next();
    const context = await browser.newContext();
    const page = await context.newPage();
    const observed = await watchPairing(page);
    const gate = routeBarrier();
    const firstPost = routeBarrier();
    await page.route(
      phase === 'SDK import' ? '**/sdk/remote-v1.js' : '**/sdk/pair-offer',
      async (route) => {
        gate.arrive();
        await gate.held;
        await route.continue();
      },
    );
    await page.route('**/r/*/pair', async (route) => {
      if (observed.posts.length === 1) {
        firstPost.arrive();
        await firstPost.held;
      }
      await route.continue();
    });
    try {
      await page.goto(offer.link as string, { waitUntil: 'domcontentloaded' });
      await gate.reached;
      await earlyPairingAttempt(page);
      await expect(page.getByRole('button', { name: 'Pair', exact: true })).toBeDisabled();
      await expect(page.locator('#pair')).toBeVisible();
      await expect(page.locator('#name')).toHaveValue('Readiness browser');
      expect(page.url()).toBe(`${origin}/pair`);
      expect(observed.posts).toEqual([]);
      expect(await observed.violations()).toEqual([]);
      await captureState(
        page,
        `pair-initializing-${phase === 'SDK import' ? 'sdk' : 'offer'}`,
        'Disabled Pair, retained name, visible reason, shared header',
      );
      gate.release();
      await expect(page.getByRole('button', { name: 'Pair', exact: true })).toBeEnabled();
      await captureState(page, 'pair-ready', 'Ready label, field association, enabled action');
      await page.locator('#name').focus();
      await captureState(page, 'pair-field-focus', 'Field keyboard focus, no clipping');
      await page.keyboard.press('Tab');
      await expect(page.getByRole('button', { name: 'Pair', exact: true })).toBeFocused();
      await captureState(page, 'pair-action-focus', 'Pair keyboard focus, retained name');
      await page.keyboard.press('Enter');
      await firstPost.reached;
      expect(observed.posts).toHaveLength(1);
      expect(JSON.parse(observed.posts[0]!).name).toBe('Readiness browser');
      firstPost.release();
      const candidate = await events.next();
      expect(candidate.event).toBe('candidate');
      await expect(page.locator('#words')).toHaveText((candidate.words as string[]).join(' '));
      await captureState(page, 'pair-waiting', 'Terminal words, explicit Waiting, retained header');
      pair.stdin.write('confirm\n');
      expect((await events.next()).reason).toBe('paired');
      await expect(page.locator('#status')).toHaveText(
        'This browser is paired. You can close this page.',
      );
      await captureState(page, 'pair-paired', 'Success word/mark and ordinary return link');
      await expect(page.getByRole('link', { name: 'Return to Remote and connect' })).toBeFocused();
      await captureState(
        page,
        'pair-return-focus',
        'Return link keyboard focus, native navigation ownership',
      );
      // Pending polls repeat this exact enrollment; there was only one initial submission.
      expect(new Set(observed.posts).size).toBe(1);
      expect(await observed.violations()).toEqual([]);
    } finally {
      gate.release();
      firstPost.release();
      await context.close();
    }
  });
}

for (const failure of ['SDK abort', 'offer abort', 'offer refused', 'offer malformed']) {
  test(`pairing remains unavailable after ${failure}`, async () => {
    pair = spawn(BINARY, ['pair', '--json'], { env });
    const events = lines(pair);
    const offer = await events.next();
    const context = await browser.newContext();
    const page = await context.newPage();
    const observed = await watchPairing(page);
    const failed = routeBarrier();
    await page.route(
      failure === 'SDK abort' ? '**/sdk/remote-v1.js' : '**/sdk/pair-offer',
      async (route) => {
        if (failure.endsWith('abort')) await route.abort('failed');
        else
          await route.fulfill({
            status: failure === 'offer refused' ? 404 : 200,
            contentType: 'application/json',
            body: '{}',
          });
        failed.arrive();
      },
    );
    try {
      await page.goto(offer.link as string);
      await failed.reached;
      if (failure !== 'SDK abort') {
        await expect(page.locator('#status')).toHaveAttribute('data-state', 'blocked');
        await expect(page.locator('#pair')).toBeHidden();
      } else {
        await earlyPairingAttempt(page);
      }
      await captureState(
        page,
        `pair-${failure.replaceAll(' ', '-').toLowerCase()}`,
        'Failed initialization remains unavailable; zero submissions',
      );
      // Hidden controls still retain the disabled property.
      expect(
        await page
          .locator('#pair button')
          .evaluate((button) => (button as HTMLButtonElement).disabled),
      ).toBe(true);
      expect(observed.posts).toEqual([]);
      expect(await observed.violations()).toEqual([]);
    } finally {
      await context.close();
    }
  });
}

test('a browser pairs, gets a door session and certifies only its own extension', async () => {
  pair = spawn(BINARY, ['pair', '--json', '--talk'], { env });
  const events = lines(pair);
  const offer = await events.next();
  const link = offer.link as string;
  const code = link.split('#')[1]!;
  expect(new URL(link).pathname).toBe('/pair');
  expect(code).toMatch(/^[A-Z2-7]{26}$/);
  expect(link.length).toBeLessThanOrEqual(59);
  const context = await browser.newContext();
  const requested: string[] = [];
  context.on('request', (request) => requested.push(request.url()));
  const page = await context.newPage();
  await page.route('**/sdk/remote-v1.js', async (route) => {
    // The synchronous bootstrap must erase the code before the SDK is even requested.
    expect(await page.evaluate(() => location.hash)).toBe('');
    await route.continue();
  });
  await page.goto(link);
  // The page removed the fragment before anything else ran.
  await page.waitForFunction(() => location.hash === '');
  await page.fill('#name', 'E2E browser');
  await page.click('button');
  await expect(page.locator('#words')).toBeVisible();
  const candidate = await events.next();
  expect(candidate.event).toBe('candidate');
  await expect(page.locator('#words')).toHaveText((candidate.words as string[]).join(' '));
  pair.stdin.write('confirm\n');
  expect((await events.next()).reason).toBe('paired');
  await expect(page.locator('#status')).toHaveText(
    'This browser is paired. You can close this page.',
  );
  await expect(page.locator('#mark')).toHaveText('✓');
  await expectStateColor(page, 'light', 'working');
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
    'ReopenSessionError',
    'budget',
    'certifyKey',
    'landingPage',
    'management',
    'operations',
    'pairingPage',
    'reopenSession',
    'transportUrl',
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
  // Two pages of one context share the device cookie but retain independent transports/lanes.
  const otherTab = await context.newPage();
  await otherTab.goto(`${mounts}colab/other`);
  const attach = async (tab: Page, reuseSession = false) =>
    tab.evaluate(async (reuseSession) => {
      const sdk = (await import(
        '/sdk/remote-v1.js' as string
      )) as typeof import('../src/browser.js');
      const session = reuseSession
        ? (window as unknown as { remoteTest: { session: import('../src/device.js').Session } })
            .remoteTest.session
        : await sdk.reopenSession();
      const url = sdk.transportUrl(
        session,
        location.href.replace('http:', 'ws:').replace(/\/[^/]*$/, '/sync'),
      );
      const socket = new WebSocket(url);
      await new Promise<void>((resolve, reject) => {
        socket.onopen = () => resolve();
        socket.onerror = () => reject(new Error('Transport refused.'));
      });
      Object.assign(window, { remoteTest: { session, socket, remote: sdk.operations(session) } });
      return session.sessionId;
    }, reuseSession);
  const firstSession = await attach(app);
  const secondSession = await attach(otherTab);
  expect(firstSession).not.toBe(secondSession);
  const list = async (tab: Page) =>
    tab.evaluate(async () => {
      const state = (
        window as unknown as {
          remoteTest: { remote: import('../src/operations.js').RemoteOperations };
        }
      ).remoteTest;
      return state.remote.listAgents();
    });
  expect(await list(app)).toEqual(await list(otherTab));
  // Last-close keeps the same session usable inside the inactivity grace.
  const closeTransport = async () =>
    app.evaluate(async () => {
      const state = (window as unknown as { remoteTest: { socket: WebSocket } }).remoteTest;
      await new Promise<void>((resolve) => {
        state.socket.onclose = () => resolve();
        state.socket.close();
      });
    });
  await closeTransport();
  expect(await list(app)).toHaveLength(1);
  expect(await list(otherTab)).toHaveLength(1);
  expect(await attach(app, true)).toBe(firstSession);
  expect(await list(app)).toHaveLength(1);
  expect(await list(otherTab)).toHaveLength(1);
  // Explicit reopen still creates an independent session; no send is retried.
  await closeTransport();
  const reopenedSession = await attach(app);
  expect(reopenedSession).not.toBe(firstSession);
  expect(reopenedSession).not.toBe(secondSession);
  expect(await list(app)).toHaveLength(1);
  // Closing one tab drops its transport; the surviving tab remains usable.
  await otherTab.close();
  expect(await list(app)).toHaveLength(1);
  expect(new URL(app.url()).search).toBe('');

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
  // Real-door continuity: one opaque admission is retried, never the original send.
  await app.evaluate(async () => {
    const sdk = (await import('/sdk/remote-v1.js' as string)) as typeof import('../src/browser.js');
    (
      globalThis as unknown as { continuitySession: import('../src/device.js').Session }
    ).continuitySession = await sdk.reopenSession();
  });
  let refusedOpen = false;
  let continuityOpens = 0;
  const interruptOpen = async (route: import('@playwright/test').Route) => {
    if (JSON.parse(route.request().postData()!).operation === 'session.open') {
      continuityOpens++;
      if (!refusedOpen) {
        refusedOpen = true;
        await route.fulfill({ status: 404, body: '' });
        return;
      }
    }
    await route.continue();
  };
  await app.route('**/r/*/append', interruptOpen);
  try {
    const continued = await app.evaluate(async (id) => {
      const sdk = (await import(
        '/sdk/remote-v1.js' as string
      )) as typeof import('../src/browser.js');
      const previous = (
        globalThis as unknown as { continuitySession: import('../src/device.js').Session }
      ).continuitySession;
      const replacement = await sdk.reopenSession(previous, { retry: 'bounded' });
      const remote = sdk.operations(replacement);
      return {
        differentSession: previous.sessionId !== replacement.sessionId,
        agents: await remote.listAgents(),
        original: await remote.operation(id),
      };
    }, asked.input.operationId);
    expect(continued.differentSession).toBe(true);
    expect(continued.agents).toEqual(asked.agents);
    expect(continued.original).toEqual(asked.sent);
    expect(continuityOpens).toBe(2);
    const calls = (await readFile(join(root, 'core-calls.jsonl'), 'utf8'))
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line) as { operation: string });
    expect(calls.filter((call) => call.operation === 'dispatch.create')).toHaveLength(1);
  } finally {
    await app.unroute('**/r/*/append', interruptOpen);
  }
  // A signed refused open leaves the attached page usable.
  execFileSync(BINARY, ['settings', 'sessions-per-device', '1', '--json'], { env });
  const capacity = await app.evaluate(async () => {
    const sdk = (await import('/sdk/remote-v1.js' as string)) as typeof import('../src/browser.js');
    try {
      const previous = (
        globalThis as unknown as { continuitySession: import('../src/device.js').Session }
      ).continuitySession;
      await sdk.reopenSession(previous, { retry: 'bounded' });
      throw new Error('Attached capacity unexpectedly admitted another Session.');
    } catch (error) {
      if (!(error instanceof sdk.ReopenSessionError) || !(error.cause instanceof sdk.RefusalError))
        throw error;
      return {
        reason: error.reason,
        code: error.cause.code,
        limit: error.cause.limit,
        settingsUrl: error.cause.settingsUrl,
      };
    }
  });
  expect(capacity).toEqual({
    reason: 'capacity',
    code: 'REMOTE_SESSION_LIMIT',
    limit: 1,
    settingsUrl: `${origin}/settings`,
  });
  expect(await list(app)).toHaveLength(1);
  execFileSync(BINARY, ['settings', 'sessions-per-device', '8', '--json'], { env });
  // Refresh the detached victim's shared cookie before reload; its lifetime gap is #2576.
  await app.evaluate(async () => {
    const sdk = (await import('/sdk/remote-v1.js' as string)) as typeof import('../src/browser.js');
    await sdk.reopenSession();
  });
  await app.reload();
  expect(JSON.parse((await app.locator('#context').textContent())!)).toMatchObject({
    deviceId: device.deviceId,
  });
  await exited(pair);
  // Model a previously shipped schema5 door and browser address. The real restart migrates
  // the machine path; the stored origin-bound key/grant must reopen without pairing again.
  const oldMounts = mounts;
  const legacyPrefix = '/r/0123456789abcdef0123456789abcdef';
  await app.evaluate(async (address) => {
    const database = await new Promise<IDBDatabase>((resolve) => {
      const request = indexedDB.open('tmt-remote', 1);
      request.onsuccess = () => resolve(request.result);
    });
    const stored = await new Promise<Record<string, unknown>>((resolve) => {
      const request = database.transaction('device').objectStore('device').get('device');
      request.onsuccess = () => resolve(request.result as Record<string, unknown>);
    });
    await new Promise<void>((resolve, reject) => {
      const transaction = database.transaction('device', 'readwrite');
      transaction
        .objectStore('device')
        .put({ ...stored, paired: { ...(stored.paired as object), address } }, 'device');
      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(transaction.error);
    });
    database.close();
  }, `${origin}${legacyPrefix}`);
  serve.kill('SIGTERM');
  await exited(serve);
  execFileSync(
    'python3',
    [
      '-c',
      "import sqlite3,sys; db=sqlite3.connect(sys.argv[1]); db.execute('UPDATE machine SET route_prefix=?', (sys.argv[2],)); db.execute('DELETE FROM _migrations WHERE version>=6'); db.execute('DROP TABLE settings_designation'); db.execute('DROP TABLE management_receipts'); db.execute('DROP TABLE sessions'); db.execute('CREATE TABLE sessions(client_id TEXT PRIMARY KEY REFERENCES grants(client_id),session_id TEXT NOT NULL UNIQUE,window_id TEXT NOT NULL,grant_revision INTEGER NOT NULL,next_client_sequence TEXT NOT NULL,next_server_sequence TEXT NOT NULL)'); db.execute('ALTER TABLE operations DROP COLUMN grant_revision'); db.execute('ALTER TABLE operations DROP COLUMN session_id'); db.commit(); db.close()",
      join(root, 'state/remote/remote.db'),
      legacyPrefix,
    ],
    { timeout: 10_000 },
  );
  serve = spawn(BINARY, ['serve', '--json'], { env });
  const restarted = await lines(serve).next();
  expect(new URL(restarted.address as string).origin).toBe(origin);
  expect(new URL(restarted.address as string).pathname).toMatch(/^\/r\/[a-z2-7]{16}$/);
  mounts = `${restarted.address as string}/x/`;
  expect(mounts).not.toBe(oldMounts);
  expect((await app.goto(`${origin}${legacyPrefix}/x/colab/home`))?.status()).toBe(404);
  await app.goto(`${mounts}colab/home`);
  await expect(app.locator('#context')).toHaveText('none');
  await app.evaluate(async () => {
    const sdk = (await import('/sdk/remote-v1.js' as string)) as typeof import('../src/browser.js');
    await sdk.reopenSession();
  });
  await app.reload();
  expect(JSON.parse((await app.locator('#context').textContent())!)).toMatchObject({
    deviceId: device.deviceId,
  });
  expect((await page.request.get(`${origin}/sdk/pair-offer`)).status()).toBe(404);
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
test('browser pages consume shared presentation in both schemes and fit desktop and mobile', async () => {
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
        const inspect = async (state: 'landing' | 'pairing' | 'confirmation' | 'error') => {
          await expect(page.locator('.header-title')).toHaveText(
            state === 'error'
              ? 'Page unavailable'
              : state === 'landing'
                ? 'Remote connection'
                : 'Pair a browser',
          );
          await expect(page.locator('.state-mark')).toHaveText(
            { landing: '○', pairing: '◆', confirmation: '◆', error: '✗' }[state]!,
          );
          const look = await page.evaluate(() => {
            const sheet = document.querySelector('.sheet')!;
            const style = getComputedStyle(sheet);
            return {
              header: (() => {
                const header = document.querySelector('.header')!;
                const box = header.getBoundingClientRect();
                const part = (selector: string) => {
                  const element = document.querySelector(selector)!;
                  const font = getComputedStyle(element);
                  return {
                    size: font.fontSize,
                    weight: font.fontWeight,
                    left: element.getBoundingClientRect().left,
                  };
                };
                return {
                  width: box.width,
                  height: box.height,
                  top: box.top,
                  position: getComputedStyle(header).position,
                  rule: getComputedStyle(header).borderBottomWidth,
                  background: getComputedStyle(header).backgroundColor,
                  mark: part('.header-mark'),
                  wordmark: part('.header-wordmark'),
                  title: part('.header-title'),
                  markViewBox: document.querySelector('.header-mark')!.getAttribute('viewBox'),
                  markPaths: document.querySelectorAll('.header-mark path').length,
                  wordmarkText: document.querySelector('.header-wordmark')!.textContent,
                  headings: document.querySelectorAll('h1').length,
                };
              })(),
              markAboveEyebrow:
                document.querySelector('.tmt-ui-notice-mark')!.getBoundingClientRect().bottom <=
                (document.querySelector('.eyebrow') ??
                  document.querySelector('#heading'))!.getBoundingClientRect().top,
              paper: getComputedStyle(document.body).backgroundColor,
              sheet: style.backgroundColor,
              text: getComputedStyle(document.body).color,
              shadow: style.boxShadow,
              radius: style.borderRadius,
              border: style.borderTopWidth,
              opacity: style.opacity,
              overflow: document.documentElement.scrollWidth > innerWidth,
              // Playwright restores the input caret with an empty style attribute.
              inline: document.querySelectorAll('[style]:not([style=""]), style, script:not([src])')
                .length,
            };
          });
          expect(look).toMatchObject({
            paper: tokenRgb('surface', 'paper', colorScheme),
            sheet: tokenRgb('surface', 'sheet', colorScheme),
            text: tokenRgb('color', 'text', colorScheme),
            markAboveEyebrow: true,
            radius: '0px',
            border: '1px',
            opacity: '1',
            overflow: false,
            inline: 0,
          });
          expect(look.shadow).toBe('none');
          await expectStateColor(page, colorScheme, state === 'error' ? 'blocked' : 'waiting');
          // Colab's header, from the same tokens: mark, product, then the page title.
          const header = look.header;
          expect(header).toMatchObject({
            width,
            height: parseFloat(headerTokens[width < 480 ? 'compact-height' : 'height']!),
            top: 0,
            position: 'fixed',
            rule: '1px',
            background: tokenRgb('surface', 'sheet', colorScheme),
            markViewBox: '0 0 200 200',
            markPaths: 6,
            wordmarkText: 'Remote',
            headings: 1,
          });
          expect(header.mark).toMatchObject({
            size: headerTokens['mark-size'],
            weight: headerTokens['mark-weight'],
          });
          expect(header.wordmark).toMatchObject({
            size: headerTokens['wordmark-size'],
            weight: headerTokens['wordmark-weight'],
          });
          expect(header.title).toMatchObject({
            size: headerTokens['title-size'],
            weight: headerTokens['title-weight'],
          });
          expect(header.mark.left).toBeLessThan(header.wordmark.left);
          expect(header.wordmark.left).toBeLessThan(header.title.left);
          await captureState(
            page,
            state,
            'Shared header/notice, current roles, one window scrollbar',
          );
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
        await expect(page.locator('#heading')).toHaveText('This page is unavailable');
        await inspect('error');

        pair = spawn(BINARY, ['pair', '--json'], { env });
        const events = lines(pair);
        const offer = await events.next();
        await page.goto(offer.link as string);
        await expect(page.getByRole('button', { name: 'Pair', exact: true })).toBeEnabled();
        await inspect('pairing');
        expect(await page.locator('#mark').evaluate((mark) => getComputedStyle(mark).color)).toBe(
          tokenRgb('color', 'waiting', colorScheme),
        );
        // Keyboard-only submission exercises the visible focus and form behavior.
        await page.locator('#name').focus();
        await page.keyboard.press('Tab');
        await expect(page.getByRole('button', { name: 'Pair', exact: true })).toBeFocused();
        await page.keyboard.press('Enter');
        const candidate = await events.next();
        expect(candidate.event).toBe('candidate');
        await expect(page.locator('#words')).toHaveText((candidate.words as string[]).join(' '));
        await expect(page.locator('#status')).toHaveAttribute('data-state', 'waiting');
        await expect(page.locator('.words-label')).toHaveText('Words to compare');
        await expect(page.locator('#status')).toHaveText(
          'Waiting for confirmation in your terminal. Compare these words and confirm only if they match.',
        );
        const confirmation = await page.evaluate(() => ({
          instruction: getComputedStyle(document.getElementById('status')!).color,
          body: getComputedStyle(document.body).color,
          mark: getComputedStyle(document.getElementById('mark')!).color,
          wordsSize: parseFloat(getComputedStyle(document.getElementById('words')!).fontSize),
          labelInside: document
            .getElementById('words')!
            .contains(document.querySelector('.words-label')),
        }));
        expect(confirmation.instruction).toBe(confirmation.body);
        expect(confirmation.mark).toBe(tokenRgb('color', 'waiting', colorScheme));
        expect(confirmation.wordsSize).toBeGreaterThanOrEqual(24);
        expect(confirmation.labelInside).toBe(false);
        await inspect('confirmation');
        pair.stdin.write('refuse\n');
        expect((await events.next()).reason).toBe('refused');
        await exited(pair);
        pair = undefined;
        await expect(page.locator('#status')).toHaveAttribute('data-state', 'blocked');
        await expect(page.locator('#status')).toHaveText(
          'Pairing did not complete. Run tmt remote pair again for a new link.',
        );
        await expect(page.locator('#mark')).toHaveText('✗');
        await expectStateColor(page, colorScheme, 'blocked');
        await captureState(page, 'pair-failed', 'Failure words, no retry action');
        expect((await page.goto(`${origin}/pair/abc`))?.status()).toBe(404);
        await page.goto(`${origin}/pair#BAD`);
        await expect(page.locator('#status')).toHaveAttribute('data-state', 'blocked');
        await expect(page.locator('#pair')).toBeHidden();
        await captureState(page, 'pair-offer-unavailable', 'Unavailable offer, hidden form');
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

test('the native entry checks once and presents all seven evidenced states without sending work', async () => {
  const context = await browser.newContext({
    locale: 'en-US',
    timezoneId: 'Asia/Tokyo',
    permissions: ['clipboard-read', 'clipboard-write'],
  });
  const page = await context.newPage();
  const captureEntry = async (state: string): Promise<void> => {
    await expect(page.locator('#notice')).toHaveAttribute(
      'data-tone',
      state === 'connected'
        ? 'working'
        : ['not-paired', 'checking', 'different-machine'].includes(state)
          ? 'waiting'
          : 'blocked',
    );
    await expect(page.locator('#command-pair, #command-devices, #command-status')).toHaveCount(0);
    const commands = page.locator('.entry-steps:visible code');
    const expected =
      state === 'not-accepted' ? 2 : ['checking', 'connected'].includes(state) ? 0 : 1;
    await expect(commands).toHaveCount(expected);
    expect(await commands.evaluateAll((nodes) => nodes.map((node) => node.textContent))).toEqual(
      state === 'not-accepted'
        ? ['tmt remote devices', 'tmt remote pair']
        : state === 'unconfirmed'
          ? ['tmt remote status']
          : expected
            ? ['tmt remote pair']
            : [],
    );
    expect(
      await commands.evaluateAll((nodes) =>
        nodes.every(
          (node) =>
            node.nextElementSibling?.matches('button.tmt-ui-command-copy') &&
            node.nextElementSibling.textContent?.trim() === 'Copy',
        ),
      ),
    ).toBe(true);
    expect(
      await commands.evaluateAll((nodes) =>
        nodes.every((node) => {
          const button = node.nextElementSibling!;
          const text = node.getBoundingClientRect();
          const action = button.getBoundingClientRect();
          const block = node.parentElement!;
          const css = getComputedStyle(block);
          const verticalSpace =
            parseFloat(css.paddingTop) +
            parseFloat(css.paddingBottom) +
            parseFloat(css.borderTopWidth) +
            parseFloat(css.borderBottomWidth);
          return (
            Math.abs(text.top + text.height / 2 - action.top - action.height / 2) < 1 &&
            css.paddingTop === css.paddingBottom &&
            Math.abs(
              block.getBoundingClientRect().height -
                Math.max(text.height, action.height) -
                verticalSpace,
            ) < 1
          );
        }),
      ),
    ).toBe(true);
    for (const command of await commands.allTextContents()) {
      const action = page.getByRole('button', { name: `Copy ${command}`, exact: true });
      await expect(action).toHaveCount(1);
      await expect(action).toHaveText('Copy');
    }
    expect(
      await commands.evaluateAll((nodes) =>
        nodes.every((node) => {
          const group = node.parentElement!;
          return (
            group.matches('.entry-command') &&
            (group.nextSibling?.textContent?.trim() ?? '') === '' &&
            (!group.closest('#steps-different') ||
              group.previousSibling?.textContent?.trim() ===
                'To use this machine too, run this on it:')
          );
        }),
      ),
    ).toBe(true);
    expect(
      await page.locator('.tmt-ui-action:visible').evaluateAll((nodes) =>
        nodes.every((node) => {
          const style = getComputedStyle(node);
          return (
            style.fontFamily === getComputedStyle(document.body).fontFamily &&
            style.fontWeight === '400'
          );
        }),
      ),
    ).toBe(true);
    if (state === 'not-paired') {
      await expect(page.locator('#status')).toBeHidden();
      await expect(page.locator('#pairing-note')).toHaveText(
        'Pair links are private and work once.',
      );
      await expect(page.locator('#pairing-note')).toBeVisible();
      expect(
        await page
          .locator('#pairing-note')
          .evaluate(
            (node) =>
              !!(
                document.querySelector('#steps-missing')!.compareDocumentPosition(node) &
                Node.DOCUMENT_POSITION_FOLLOWING
              ),
          ),
      ).toBe(true);
    } else await expect(page.locator('#pairing-note')).toBeHidden();
    expect(
      await page
        .locator('.tmt-ui-command-feedback')
        .first()
        .evaluate((node) => {
          const style = getComputedStyle(node);
          return [style.marginTop, style.marginBottom];
        }),
    ).toEqual(['0px', '0px']);
    await captureState(
      page,
      `entry-${state}`,
      'One evidenced state, needed next steps, Details and aperture header',
      [1440, 390, 320],
    );
    await page.setViewportSize({ width: 320, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
  };
  let mountsRead = 0,
    admissions = 0,
    observations = 0;
  const requests: string[] = [];
  page.on('request', (request) => {
    requests.push(request.url());
    if (new URL(request.url()).pathname === '/sdk/mount') mountsRead++;
    const body = request.postData();
    if (body && request.url().endsWith('/append')) {
      const operation = JSON.parse(body).operation;
      if (operation === 'session.open') admissions++;
      if (operation === 'capabilities') observations++;
    }
  });
  const counts = () => ({ mounts: mountsRead, admissions, observations });
  const reset = () => {
    mountsRead = 0;
    admissions = 0;
    observations = 0;
  };
  await page.goto(origin);
  await expect(page.locator('#state-label')).toHaveText('Not paired');
  expect(counts()).toEqual({ mounts: 0, admissions: 0, observations: 0 });
  await captureEntry('not-paired');
  await page.getByRole('button', { name: 'Copy tmt remote pair', exact: true }).click();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('tmt remote pair');
  await expect(page.locator('.entry-steps:visible .tmt-ui-command-feedback')).toHaveText('Copied.');
  pair = spawn(BINARY, ['pair', '--json'], { env });
  const events = lines(pair);
  const offer = await events.next();
  const code = (offer.link as string).split('#')[1]!;
  await page.goto(offer.link as string);
  await page.fill('#name', 'Entry browser');
  await page.getByRole('button', { name: 'Pair', exact: true }).click();
  const candidate = await events.next();
  await expect(page.locator('#words')).toHaveText((candidate.words as string[]).join(' '));
  pair.stdin.write('confirm\n');
  const paired = await events.next();
  expect(paired.reason).toBe('paired');
  await exited(pair);
  expect(pair.exitCode).toBe(0);
  await expect(page.locator('#entry-link')).toBeVisible();
  expect(requests.some((url) => url.includes(code))).toBe(false);
  const original = await page.evaluate(async () => {
    const database = await new Promise<IDBDatabase>((resolve) => {
      const request = indexedDB.open('tmt-remote', 1);
      request.onsuccess = () => resolve(request.result);
    });
    const record = await new Promise<{
      paired: { clientId: string; machineId: string; machinePublicKey: Uint8Array };
    }>((resolve) => {
      const request = database.transaction('device').objectStore('device').get('device');
      request.onsuccess = () => resolve(request.result);
    });
    database.close();
    return {
      clientId: record.paired.clientId,
      machineId: record.paired.machineId,
      machinePublicKey: Array.from(record.paired.machinePublicKey),
    };
  });
  async function changePin(machineId: string, pin: number[]): Promise<void> {
    await page.evaluate(
      async ({ machineId, pin }) => {
        const database = await new Promise<IDBDatabase>((resolve) => {
          const request = indexedDB.open('tmt-remote', 1);
          request.onsuccess = () => resolve(request.result);
        });
        const transaction = database.transaction('device', 'readwrite');
        const store = transaction.objectStore('device');
        const request = store.get('device');
        request.onsuccess = () => {
          const record = request.result;
          record.paired.machineId = machineId;
          record.paired.machinePublicKey = new Uint8Array(pin);
          store.put(record, 'device');
        };
        await new Promise<void>((resolve, reject) => {
          transaction.oncomplete = () => resolve();
          transaction.onerror = () => reject(transaction.error);
        });
        database.close();
      },
      { machineId, pin },
    );
  }

  const gate = routeBarrier();
  const continued = routeBarrier();
  const hold = async (route: import('@playwright/test').Route) => {
    gate.arrive();
    await gate.held;
    try {
      await route.continue();
    } finally {
      continued.arrive();
    }
  };
  await page.route('**/sdk/mount', hold);
  reset();
  try {
    await page.goto(origin, { waitUntil: 'domcontentloaded' });
    await gate.reached;
    await expect(page.locator('#state-label')).toHaveText('Checking');
    await expect(page.locator('#check')).toBeDisabled();
    await captureEntry('checking');
  } finally {
    gate.release();
    await continued.reached;
    await page.unroute('**/sdk/mount', hold);
  }
  await expect(page.locator('#heading')).toHaveText('This browser can use Remote');
  expect(counts()).toEqual({ mounts: 1, admissions: 1, observations: 1 });
  await expect(page.locator('#status')).toHaveText('Open your app from its link in this browser.');
  await captureEntry('connected');
  await page.locator('summary').click();
  await expect(page.locator('#machine-id')).toHaveText(original.machineId.slice(0, 8));
  await expect(page.locator('#checked-time')).not.toHaveText(/\d{4}-\d{2}-\d{2}T/);
  await captureState(page, 'entry-details', 'Viewer-local time and short machine ID under Details');
  await page.getByRole('button', { name: 'Check again' }).focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('#heading')).toHaveText('This browser can use Remote');
  await expect(page.locator('#check')).toBeEnabled();
  expect(counts()).toEqual({ mounts: 1, admissions: 1, observations: 2 });
  await expect(page.locator('#check')).toBeFocused();
  await captureState(page, 'entry-check-focus', 'Manual keyboard check retains visible focus');
  // A deterministic expired-session response on the reused lane reopens once in this click.
  let expiredRead = false;
  const expireRead = async (route: import('@playwright/test').Route) => {
    const operation = JSON.parse(route.request().postData()!).operation;
    if (operation === 'capabilities' && !expiredRead) {
      expiredRead = true;
      await route.fulfill({ status: 404, body: '' });
    } else await route.continue();
  };
  await page.route('**/r/*/append', expireRead);
  try {
    await page.getByRole('button', { name: 'Check again' }).click();
    await expect(page.locator('#heading')).toHaveText('This browser can use Remote');
    await expect(page.locator('#check')).toBeEnabled();
    expect(expiredRead).toBe(true);
    expect(counts()).toEqual({ mounts: 2, admissions: 2, observations: 4 });
  } finally {
    await page.unroute('**/r/*/append', expireRead);
  }
  const beforeGrants = execFileSync(
    'python3',
    [
      '-c',
      'import sqlite3,sys,json;d=sqlite3.connect(sys.argv[1]);print(json.dumps(d.execute("select * from grants").fetchall(),default=lambda value: value.hex()))',
      join(root, 'state/remote/remote.db'),
    ],
    { encoding: 'utf8' },
  );

  await changePin(crypto.randomUUID(), original.machinePublicKey);
  reset();
  await page.goto(origin);
  await expect(page.locator('#state-label')).toHaveText('Different machine');
  expect(counts()).toEqual({ mounts: 1, admissions: 0, observations: 0 });
  await captureEntry('different-machine');

  await changePin(original.machineId, original.machinePublicKey.slice(0, 31));
  reset();
  await page.goto(origin);
  await expect(page.locator('#state-label')).toHaveText('Pairing unreadable');
  expect(counts()).toEqual({ mounts: 0, admissions: 0, observations: 0 });
  await captureEntry('pairing-unreadable');

  await changePin(original.machineId, original.machinePublicKey);
  reset();
  await page.route('**/sdk/mount', (route) => route.abort('failed'));
  await page.goto(origin);
  await expect(page.locator('#state-label')).toHaveText("Can't reach Remote");
  expect(counts()).toEqual({ mounts: 1, admissions: 0, observations: 0 });
  await captureEntry('unconfirmed');
  await page.unroute('**/sdk/mount');
  // Clipboard denial reports a selectable fallback rather than claiming a copy.
  await page.evaluate(() => {
    navigator.clipboard.writeText = async () => {
      throw new Error('fixture denied');
    };
  });
  await page.getByRole('button', { name: 'Copy tmt remote status', exact: true }).click();
  await expect(page.locator('.entry-steps:visible .tmt-ui-command-feedback')).toHaveText(
    'Copy failed. Select the command and copy it manually.',
  );
  const changed = [...original.machinePublicKey];
  changed[0] = changed[0]! ^ 1;
  await changePin(original.machineId, changed);
  reset();
  await page.goto(origin);
  await expect(page.locator('#state-label')).toHaveText("Can't reach Remote");
  expect(counts()).toEqual({ mounts: 1, admissions: 1, observations: 0 });
  await changePin(original.machineId, original.machinePublicKey);

  // One real private-fixture session slot makes the next tab's admission deterministically evict this check.
  execFileSync(BINARY, ['settings', 'sessions-per-device', '1', '--json'], { env });
  // Hold the actual signed read before admission; the next session evicts this one.
  const reading = routeBarrier();
  const readContinued = routeBarrier();
  const holdRead = async (route: import('@playwright/test').Route) => {
    if (JSON.parse(route.request().postData()!).operation !== 'capabilities') {
      await route.continue();
      return;
    }
    reading.arrive();
    await reading.held;
    try {
      await route.continue();
    } finally {
      readContinued.arrive();
    }
  };
  await page.route('**/r/*/append', holdRead);
  reset();
  const other = await context.newPage();
  try {
    await page.goto(origin, { waitUntil: 'domcontentloaded' });
    await reading.reached;
    await other.goto(`${mounts}colab/`);
    await other.evaluate(async () => {
      const sdkUrl = '/sdk/remote-v1.js';
      const sdk = await import(sdkUrl);
      await sdk.reopenSession();
    });
  } finally {
    reading.release();
    await readContinued.reached;
    await page.unroute('**/r/*/append', holdRead);
    await other.close();
  }
  await expect(page.locator('#state-label')).toHaveText('Not accepted');
  expect(counts()).toEqual({ mounts: 1, admissions: 1, observations: 1 });
  await captureEntry('not-accepted');
  expect(
    execFileSync(
      'python3',
      [
        '-c',
        'import sqlite3,sys,json;d=sqlite3.connect(sys.argv[1]);print(json.dumps(d.execute("select * from grants").fetchall(),default=lambda value: value.hex()))',
        join(root, 'state/remote/remote.db'),
      ],
      { encoding: 'utf8' },
    ),
  ).toBe(beforeGrants);

  const revoke = spawn(BINARY, ['devices', 'revoke', original.clientId, '--json'], { env });
  expect((await lines(revoke).next()).device).toMatchObject({
    clientId: original.clientId,
    revoked: true,
  });
  await exited(revoke);
  expect(revoke.exitCode).toBe(0);
  reset();
  await page.goto(origin);
  await expect(page.locator('#state-label')).toHaveText("Can't reach Remote");
  expect(counts()).toEqual({ mounts: 2, admissions: 1, observations: 0 });
  await context.close();
  const unavailable = await browser.newContext();
  await unavailable.addInitScript(() => {
    indexedDB.open = () => {
      throw new Error('private storage diagnostic');
    };
  });
  const unavailablePage = await unavailable.newPage();
  await unavailablePage.goto(origin);
  await expect(unavailablePage.locator('#state-label')).toHaveText('Pairing unreadable');
  await expect(unavailablePage.locator('#status')).not.toContainText('private storage diagnostic');
  await unavailable.close();
  const coreCalls = (await readFile(join(root, 'core-calls.jsonl'), 'utf8'))
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line).operation);
  expect(coreCalls.every((operation) => ['capabilities', 'storage.root'].includes(operation))).toBe(
    true,
  );
  expect(
    execFileSync(
      'python3',
      [
        '-c',
        'import sqlite3,sys;d=sqlite3.connect(sys.argv[1]);print(d.execute("select count(*) from operations").fetchone()[0],d.execute("select count(*) from entries").fetchone()[0])',
        join(root, 'state/remote/remote.db'),
      ],
      { encoding: 'utf8' },
    ).trim(),
  ).toBe('0 0');
});

test('agents list carries only verified runtime evidence through the served SDK', async () => {
  const identities = [
    {
      id: '00000000-0000-4000-8000-000000000001',
      name: 'Claude live',
      presence: 'active',
      runningDriver: 'claude',
    },
    {
      id: '00000000-0000-4000-8000-000000000002',
      name: 'Codex live',
      presence: 'active',
      runningDriver: 'codex',
    },
    {
      id: '00000000-0000-4000-8000-000000000003',
      name: 'Future',
      presence: 'active',
      runningDriver: 'future-driver',
    },
    {
      id: '00000000-0000-4000-8000-000000000004',
      name: 'Codex remembered',
      presence: 'offline',
      driver: 'codex',
    },
  ];
  await writeFile(join(root, 'agents.json'), JSON.stringify({ identities }));
  pair = spawn(BINARY, ['pair', '--json'], { env });
  const events = lines(pair);
  const offer = await events.next();
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(offer.link as string);
  await page.fill('#name', 'Driver observation browser');
  await page.click('#pair button');
  await expect(page.locator('#words')).toBeVisible();
  await events.next();
  pair.stdin.write('confirm\n');
  expect((await events.next()).reason).toBe('paired');
  await exited(pair);
  await expect(page.locator('#status')).toContainText('This browser is paired.');
  const listed = await page.evaluate(async () => {
    const sdk = (await import('/sdk/remote-v1.js' as string)) as typeof import('../src/browser.js');
    const rows = await sdk.operations(await sdk.reopenSession()).listAgents();
    return { rows, keys: rows.map((row) => Object.hasOwn(row, 'runningDriver')) };
  });
  expect(listed.keys).toEqual([true, true, false, false]);
  expect(listed.rows).toEqual(
    identities.map((row) => ({
      id: row.id,
      name: row.name,
      presence: row.presence,
      ...(row.runningDriver === 'claude' || row.runningDriver === 'codex'
        ? { runningDriver: row.runningDriver }
        : {}),
    })),
  );
  expect(
    (await readFile(join(root, 'core-list-calls.jsonl'), 'utf8'))
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line)),
  ).toEqual([['list', '--json']]);
  expect(
    execFileSync(
      'python3',
      [
        '-c',
        'import sqlite3,sys;d=sqlite3.connect(sys.argv[1]);print(d.execute("select count(*) from operations").fetchone()[0],d.execute("select count(*) from entries").fetchone()[0])',
        join(root, 'state/remote/remote.db'),
      ],
      { encoding: 'utf8' },
    ).trim(),
  ).toBe('0 0');
  await context.close();
});

test('bare pairing is read only and settings explicitly enables audited sending', async () => {
  pair = spawn(BINARY, ['pair', '--json'], { env });
  const events = lines(pair);
  const offer = await events.next();
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(offer.link as string);
  await page.fill('#name', 'Settings browser');
  await page.click('button');
  await expect(page.locator('#words')).toBeVisible();
  await events.next();
  pair.stdin.write('confirm\n');
  expect((await events.next()).reason).toBe('paired');
  await expect(page.locator('#status')).toContainText('This browser is paired.');
  await exited(pair);
  const inventory = JSON.parse(
    execFileSync(BINARY, ['devices', '--json'], { env, encoding: 'utf8' }),
  ) as { devices: { clientId: string; scopes: string[] }[] };
  const device = inventory.devices[0]!;
  expect(device.scopes).toEqual(['agents.read', 'check.read', 'results.read', 'status.read']);
  execFileSync(BINARY, ['devices', 'designate', device.clientId, '--json'], { env });
  await page.goto(`${origin}/settings`);
  await expect(page.locator('#access')).toContainText('confirmed');
  await expect(
    page.getByRole('button', { name: 'Enable sending for Settings browser', exact: true }),
  ).toBeEnabled();
  await expect(page.locator('.device-summary').first()).toContainText('Paired · Sending off ·');
  await captureState(
    page,
    'bare-pair-sending-off',
    'Bare read-only pairing; explicit sending action',
    [1440, 320],
    ['light'],
  );
  await captureState(
    page,
    'bare-pair-sending-off',
    'Bare read-only pairing at narrow dark width',
    [390],
    ['dark'],
  );
  const confirmation = page.waitForEvent('dialog');
  const enable = page
    .getByRole('button', { name: 'Enable sending for Settings browser', exact: true })
    .click();
  const dialog = await confirmation;
  expect(dialog.message()).toBe('Allow Settings browser to send to its permitted agents?');
  await dialog.accept();
  await enable;
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'committed');
  await page.click('#recover');
  await expect(
    page.getByRole('button', { name: 'Disable sending for Settings browser', exact: true }),
  ).toBeEnabled();
  await expect(page.locator('.device-summary').first()).toContainText('Paired · Sending on ·');
  await captureState(
    page,
    'bare-pair-sending-on',
    'Explicit sending enabled after original-only recovery',
    [1440],
    ['light'],
  );
  await captureState(
    page,
    'bare-pair-sending-on',
    'Explicit sending enabled at narrow dark width',
    [390],
    ['dark'],
  );
  const enabled = JSON.parse(
    execFileSync(BINARY, ['devices', '--json'], { env, encoding: 'utf8' }),
  ) as { devices: { scopes: string[] }[] };
  expect(enabled.devices[0]!.scopes).toContain('talk');
  await context.close();
});

test('settings draft preserves authority, exact values, drafts and unknown self-change outcomes', async () => {
  pair = spawn(BINARY, ['pair', '--json', '--talk'], { env });
  const events = lines(pair);
  const offer = await events.next();
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(offer.link as string);
  await page.fill('#name', 'Settings browser');
  await page.click('button');
  await expect(page.locator('#words')).toBeVisible();
  await events.next();
  pair.stdin.write('confirm\n');
  expect((await events.next()).reason).toBe('paired');
  await expect(page.locator('#status')).toContainText('This browser is paired.');
  await exited(pair);
  const inventory = JSON.parse(
    execFileSync(BINARY, ['devices', '--json'], { env, encoding: 'utf8' }),
  ) as { devices: { clientId: string }[] };
  const clientId = inventory.devices[0]!.clientId;
  async function descriptions(editable: boolean): Promise<void> {
    await expect(page.locator('.device-summary').first()).toContainText(
      ' · This device · browser · Paired · ',
    );
    const guidance =
      'Read-only in this browser. Change Remote settings or manage devices with the local CLI.';
    const help = 'Default is 8. Off means unlimited. Changes apply at the next session open.';
    await expect(page.locator(`#name-${clientId}`)).toHaveAccessibleDescription(
      editable ? '' : guidance,
    );
    await expect(page.locator('#opening-form')).toHaveAccessibleDescription(
      editable ? '' : guidance,
    );
    await expect(page.locator('#limit-form')).toHaveAccessibleDescription(
      editable ? help : `${help} ${guidance}`,
    );
  }

  const loading = routeBarrier();
  const joined = routeBarrier();
  const holdSettings = async (route: import('@playwright/test').Route) => {
    loading.arrive();
    try {
      await loading.held;
      await route.continue();
    } finally {
      joined.arrive();
    }
  };
  await page.route('**/sdk/settings-v1.js', holdSettings);
  try {
    await page.goto(`${origin}/settings`, { waitUntil: 'commit' });
    await loading.reached;
    await expect(page.locator('header')).toHaveClass('header tmt-ui-header');
    await captureState(
      page,
      'settings-loading',
      'Real served initial controls while SDK entry is held',
    );
  } finally {
    loading.release();
    await joined.reached;
    await page.unroute('**/sdk/settings-v1.js', holdSettings);
  }
  await expect(page.locator('#access')).toContainText('confirmed');
  await expect(page.locator('#read-only')).toBeVisible();
  await expect(page.locator('#opening')).toBeDisabled();
  await descriptions(false);
  await captureState(
    page,
    'settings-read-only',
    'Non-designated browser, visible local CLI and disabled reasons',
  );
  await expect(page.locator('.device-summary').first()).toHaveText(
    /^Settings browser · This device · browser · Paired · Sending on · 2 live sessions · Last activity .+$/,
  );
  await expect(page.locator('#limit-value')).toHaveText('8 · default');
  execFileSync(BINARY, ['devices', 'designate', clientId, '--json'], { env });
  await page.click('#refresh');
  await expect(page.locator('#opening')).toBeEnabled();
  await expect(page.locator('#read-only')).toBeHidden();
  await descriptions(true);
  for (const theme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: theme });
    const computed = await page.locator(`#name-${clientId}`).evaluate((input) => {
      const css = getComputedStyle(input);
      return {
        color: css.color,
        background: css.backgroundColor,
        shadow: css.boxShadow,
        opacity: css.opacity,
      };
    });
    expect(computed).toEqual({
      color: tokenRgb('color', 'text', theme),
      background: tokenRgb('surface', 'sheet', theme),
      shadow: 'none',
      opacity: '1',
    });
    await expect(page.locator(`#name-${clientId}`)).toHaveAccessibleName(
      'Device name for Settings browser',
    );
    await expect(page.locator('#opening')).toHaveAccessibleName('Open a browser when pairing');
  }
  await page.emulateMedia({ colorScheme: 'light' });
  await page.locator(`#name-${clientId}`).focus();
  await page.keyboard.press('ArrowLeft');
  expect(
    await page
      .locator(`#name-${clientId}`)
      .evaluate((control) => control.matches(':focus-visible')),
  ).toBe(true);
  await captureState(
    page,
    'settings-device-keyboard-focus',
    'Shared field focus outline and stable native device control',
  );
  await page.selectOption('#opening', 'off');
  await page.locator('#opening').focus();
  await page.keyboard.press('Tab');
  await expect(page.locator('#opening-form button')).toBeFocused();
  expect(
    await page
      .locator('#opening-form button')
      .evaluate((control) => control.matches(':focus-visible')),
  ).toBe(true);
  await captureState(
    page,
    'settings-action-keyboard-focus',
    'Shared Action keyboard focus; native submit ownership',
  );
  await page.selectOption('#opening', 'on');
  await captureState(
    page,
    'settings-designated-default',
    'Designated browser; unset default8 and admitted controls',
  );
  // Explicit self scope changes use the same original-only recovery, never a resend.
  await page
    .getByRole('button', { name: 'Disable sending for Settings browser', exact: true })
    .click();
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'committed');
  await expect(page.locator('#access')).toContainText('unconfirmed');
  await page.click('#recover');
  await expect(
    page.getByRole('button', { name: 'Enable sending for Settings browser', exact: true }),
  ).toBeEnabled();
  await expect(page.locator('.device-summary').first()).toHaveText(
    /^Settings browser · This device · browser · Paired · Sending off · 1 live session · Last activity .+$/,
  );
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await captureState(
      page,
      'settings-sending-disabled',
      'Explicit sending scope disabled; other device policy and original receipt retained',
    );
  }
  const sendingConfirmation = page.waitForEvent('dialog');
  const enableSending = page
    .getByRole('button', { name: 'Enable sending for Settings browser', exact: true })
    .click();
  const sendingDialog = await sendingConfirmation;
  expect(sendingDialog.message()).toBe('Allow Settings browser to send to its permitted agents?');
  await sendingDialog.accept();
  await enableSending;
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'committed');
  await page.click('#recover');
  await expect(
    page.getByRole('button', { name: 'Disable sending for Settings browser', exact: true }),
  ).toBeEnabled();
  await expect(page.locator('.device-summary').first()).toHaveText(
    /^Settings browser · This device · browser · Paired · Sending on · 1 live session · Last activity .+$/,
  );
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await captureState(
      page,
      'settings-sending-enabled',
      'Explicit owner-settings sending enabled after confirmation and original-only recovery',
    );
  }
  await page.emulateMedia({ colorScheme: 'light' });
  // Refresh preserves the unset source and never turns default 8 into an explicit cap.
  await page.click('#refresh');
  await expect(page.locator('#refresh')).toBeEnabled();
  await descriptions(true);
  expect(
    JSON.parse(execFileSync(BINARY, ['settings', '--json'], { env, encoding: 'utf8' }))
      .sessionsPerDeviceSource,
  ).toBe('default');
  await page.selectOption('#limit-mode', 'custom');
  await page.fill('#limit-custom', '8');
  await page.click('#limit-form button');
  await expect(page.locator('#limit-value')).toHaveText('8 · settings.json');
  await captureState(
    page,
    'settings-explicit-custom',
    'Explicit custom8 differs from default source',
  );
  await page.selectOption('#opening', 'off');
  await page.click('#opening-form button');
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'committed');
  await expect(page.locator('#opening-value')).toHaveText('Off · settings.json');
  await page.selectOption('#limit-mode', 'custom');
  await page.fill('#limit-custom', '18446744073709551615');
  await page.click('#limit-form button');
  await expect(page.locator('#limit-value')).toHaveText('18446744073709551615 · settings.json');
  expect(execFileSync(BINARY, ['settings', '--json'], { env, encoding: 'utf8' })).toContain(
    '18446744073709551615',
  );
  await page.selectOption('#limit-mode', 'off');
  await page.click('#limit-form button');
  await expect(page.locator('#limit-value')).toHaveText('Off (unlimited) · settings.json');
  // Role removal fences stale admitted capabilities and keeps unsent text.
  await page.selectOption('#limit-mode', 'custom');
  await page.fill('#limit-custom', '19');
  execFileSync(BINARY, ['devices', 'undesignate', '--json'], { env });
  await page.click('#limit-form button');
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'refused');
  await expect(page.locator('#limit-custom')).toHaveValue('19');
  const roleRefresh = routeBarrier();
  const roleRefreshJoined = routeBarrier();
  let heldRoleRefresh = false;
  const holdRoleRefresh = async (route: import('@playwright/test').Route) => {
    const body = route.request().postDataJSON() as { operation?: string };
    if (body.operation !== 'remote.settings.show' || heldRoleRefresh) {
      await route.continue();
      return;
    }
    heldRoleRefresh = true;
    roleRefresh.arrive();
    try {
      await roleRefresh.held;
      await route.continue();
    } finally {
      roleRefreshJoined.arrive();
    }
  };
  await page.route('**/append', holdRoleRefresh);
  await page.click('#refresh');
  await roleRefresh.reached;
  let roleRefreshSettled = false;
  const settledRoleRefresh = (async () => {
    await expect(page.locator('#refresh')).toBeEnabled();
    await expect(page.locator('#refresh')).toHaveAttribute('aria-busy', 'false');
    roleRefreshSettled = true;
  })();
  try {
    await expect(page.locator('#refresh')).toBeDisabled();
    await expect(page.locator('#refresh')).toHaveAttribute('aria-busy', 'true');
    expect(roleRefreshSettled).toBe(false);
  } finally {
    roleRefresh.release();
    await roleRefreshJoined.reached;
    await settledRoleRefresh;
    await page.unroute('**/append', holdRoleRefresh);
  }
  await expect(page.locator('#read-only')).toBeVisible();
  await expect(page.locator('#limit-custom')).toHaveValue('19');
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'refused');
  await descriptions(false);
  await captureState(
    page,
    'settings-role-removed-draft',
    'Role removed; unsent19 draft and historical refused outcome retained',
  );
  expect(
    JSON.parse(execFileSync(BINARY, ['settings', '--json'], { env, encoding: 'utf8' }))
      .sessionsPerDevice,
  ).toBe(null);
  execFileSync(BINARY, ['devices', 'designate', clientId, '--json'], { env });
  await page.click('#refresh');
  await expect(page.locator('#read-only')).toBeHidden();
  await descriptions(true);
  await captureState(
    page,
    'settings-restored-draft',
    'Restored current access remains separate from prior refusal and unsent draft',
  );
  const captures = process.env.TMT_REMOTE_CAPTURE_DIR;
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    for (const width of [1440, 390, 320]) {
      await page.setViewportSize({ width, height: 1000 });
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await page.evaluate(() => scrollTo(0, 0));
      if (captures && width !== 320) {
        await mkdir(captures, { recursive: true });
        await page.screenshot({
          path: join(captures, `settings-${colorScheme}-${width}.png`),
          fullPage: true,
        });
      }
    }
  }
  await page.setViewportSize({ width: 1440, height: 1000 });
  let renameCalls = 0,
    revokeCalls = 0,
    opens = 0;
  const statuses: { operation: string; status: number }[] = [];
  page.on('response', (response) => {
    if (new URL(response.url()).pathname.endsWith('/append'))
      statuses.push({
        operation: response.request().postDataJSON().operation,
        status: response.status(),
      });
  });
  let recorded!: () => void, release!: () => void;
  const published = new Promise<void>((resolve) => {
    recorded = resolve;
  });
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  const recovery = routeBarrier();
  const recoveryJoined = routeBarrier();
  let recoveryHeld = false,
    holdRecovery = true;
  await page.route('**/append', async (route) => {
    const body = route.request().postDataJSON() as { operation: string };
    if (body.operation === 'session.open') {
      opens++;
      if (holdRecovery) {
        recoveryHeld = true;
        recovery.arrive();
        try {
          await recovery.held;
          await route.continue();
        } finally {
          recoveryJoined.arrive();
        }
        return;
      }
    }
    if (body.operation === 'remote.devices.rename') {
      renameCalls++;
      expect((await route.fetch()).status()).toBe(200);
      recorded();
      await held;
      await route.abort();
    } else if (body.operation === 'remote.devices.revoke') {
      revokeCalls++;
      expect((await route.fetch()).status()).toBe(200);
      await route.abort();
    } else await route.continue();
  });
  const name = page.locator(`#name-${clientId}`);
  await name.fill('Renamed browser');
  await page.locator('.device button[type=submit]').click();
  await published;
  try {
    await captureState(
      page,
      'settings-saving-self-rename',
      'Actual native effect with acknowledgement held; saving and disabled actions',
    );
    await name.fill('Unsent next name');
    await name.evaluate((input) => {
      (input as HTMLInputElement).setSelectionRange(2, 2);
    });
  } finally {
    release();
  }
  await expect(page.locator('#outcome')).toHaveText(
    'This change could not be confirmed. Check its original result before making another change.',
  );
  await expect(name).toHaveValue('Unsent next name');
  expect(await name.evaluate((input) => (input as HTMLInputElement).selectionStart)).toBe(2);
  await expect(name).toBeFocused();
  await captureState(
    page,
    'settings-unknown-name-focus',
    'Unknown original rename, retained next-name draft/caret/focus; no resend',
  );
  await name.press('Enter');
  expect(renameCalls).toBe(1);
  await expect(page.locator('#original')).toHaveText(/^Original operation [0-9a-f-]{36}$/);
  try {
    await page.click('#recover');
    await recovery.reached;
    await captureState(
      page,
      'settings-recovery-pending',
      'One fresh live admission in progress; original ID and draft are frozen, no resend',
    );
  } finally {
    holdRecovery = false;
    recovery.release();
    if (recoveryHeld) await recoveryJoined.reached;
  }
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'committed');
  await expect(page.locator('#original')).toBeEmpty();
  await expect(page.locator('#original')).toBeHidden();
  await expect(name).toHaveValue('Unsent next name');
  expect(renameCalls).toBe(1);
  expect(opens).toBe(1);
  await captureState(
    page,
    'settings-rename-recovered',
    'Fresh live Session reads committed original; next-name draft retained',
  );
  // Lost self-revoke acknowledgement: one fresh read-only admission, accurate access loss + unknown.
  const confirmation = page.waitForEvent('dialog');
  const revokeClick = page
    .locator('.device')
    .getByRole('button', { name: 'Revoke Renamed browser', exact: true })
    .click();
  const dialog = await confirmation;
  try {
    expect(dialog.type()).toBe('confirm');
    expect(dialog.message()).toBe('Revoke Renamed browser (this device)?');
  } finally {
    await dialog.accept();
  }
  await revokeClick;
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'unknown');
  await page.click('#recover');
  await expect(page.locator('#access')).toContainText('refused');
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'unknown');
  await expect(page.locator('#recover')).toBeHidden();
  await expect(page.locator('#opening')).toBeDisabled();
  await captureState(
    page,
    'settings-self-revoke-access-lost',
    'Self-revoke lost ack; current access loss and historical unknown stay separate',
  );
  expect(revokeCalls).toBe(1);
  expect(opens).toBe(2);
  // Observe completion even when the ended SDK rejects locally without an HTTP request.
  await page.evaluate(() => {
    const fixtureWindow = window as typeof window & { refreshFinished?: Promise<void> };
    const control = document.querySelector<HTMLButtonElement>('#refresh')!;
    fixtureWindow.refreshFinished = new Promise((resolve) => {
      const observer = new MutationObserver((records) => {
        if (records.some((record) => record.oldValue !== null) && !control.disabled) {
          observer.disconnect();
          resolve();
        }
      });
      observer.observe(control, {
        attributes: true,
        attributeFilter: ['disabled'],
        attributeOldValue: true,
      });
    });
  });
  await page.click('#refresh');
  await page.evaluate(async () => {
    await (window as typeof window & { refreshFinished: Promise<void> }).refreshFinished;
  });
  await expect(page.locator('#outcome'), JSON.stringify(statuses)).toHaveAttribute(
    'data-state',
    'unknown',
  );
  expect(opens).toBe(2);
  const calls = (await readFile(join(root, 'core-calls.jsonl'), 'utf8'))
    .split('\n')
    .filter(Boolean)
    .map((line) => JSON.parse(line) as { operation: string });
  expect(calls.filter((call) => call.operation === 'dispatch.create')).toHaveLength(0);
  await context.close();
});

test('settings pagination retains later-page drafts across refresh and guards navigation', async () => {
  pair = spawn(BINARY, ['pair', '--json'], { env });
  const events = lines(pair);
  const offer = await events.next();
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(offer.link as string);
  await page.fill('#name', 'Pagination browser');
  await page.click('button');
  await expect(page.locator('#words')).toBeVisible();
  await events.next();
  pair.stdin.write('confirm\n');
  expect((await events.next()).reason).toBe('paired');
  await expect(page.locator('#status')).toContainText('This browser is paired.');
  await exited(pair);
  const inventory = JSON.parse(
    execFileSync(BINARY, ['devices', '--json'], { env, encoding: 'utf8' }),
  ) as { devices: { clientId: string }[] };
  execFileSync(BINARY, ['devices', 'designate', inventory.devices[0]!.clientId, '--json'], { env });
  // Populate only the disposable fixture's Store with 52 distinct live grants.
  // The paired browser still supplies every signed read/effect; no authority is mocked.
  execFileSync('python3', [
    '-c',
    "import sqlite3,sys; db=sqlite3.connect(sys.argv[1]); db.executemany('INSERT INTO grants VALUES (?,?,?,?,?,?,?,?,?,?,?,?)', [('00000000-0000-4000-8000-%012d'%i, i.to_bytes(32,'big'),'browser',sys.argv[2],'Device %d'%i,'all','capabilities','direct',0,None,1,0) for i in range(1,53)]); db.commit(); db.close()",
    join(root, 'state/remote/remote.db'),
    origin,
  ]);
  await page.goto(`${origin}/settings`);
  await expect(page.locator('.device')).toHaveCount(25);
  await page.click('#more');
  const name = page.locator('#name-00000000-0000-4000-8000-000000000026');
  await expect(name).toBeVisible();
  await name.fill('Unsent later-page name');
  await name.evaluate((input) => (input as HTMLInputElement).setSelectionRange(3, 3));
  await page.click('#refresh');
  await expect(page.locator('#refresh')).toBeEnabled();
  await expect(name).toHaveValue('Unsent later-page name');
  expect(await name.evaluate((input) => (input as HTMLInputElement).selectionStart)).toBe(3);
  // A different committed effect also refreshes this page without resetting its forms.
  await page.selectOption('#opening', 'off');
  await page.click('#opening-form button');
  await expect(page.locator('#opening-value')).toHaveText('Off · settings.json');
  await expect(name).toHaveValue('Unsent later-page name');
  // Original-receipt recovery refresh also stays on this page after a real lost ack.
  let settingCalls = 0;
  await page.route('**/append', async (route) => {
    if (route.request().postDataJSON().operation === 'remote.settings.set') {
      settingCalls++;
      expect((await route.fetch()).status()).toBe(200);
      await route.abort();
    } else await route.continue();
  });
  await page.selectOption('#opening', 'on');
  await page.click('#opening-form button');
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'unknown');
  await expect(page.locator('#original')).toHaveText(/^Original operation [0-9a-f-]{36}$/);
  await page.click('#recover');
  await expect(page.locator('#opening-value')).toHaveText('On · settings.json');
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'committed');
  await expect(page.locator('#original')).toBeEmpty();
  await expect(page.locator('#original')).toBeHidden();
  await expect(name).toHaveValue('Unsent later-page name');
  expect(settingCalls).toBe(1);
  // Both directions refuse to discard a dirty UUID-bound form, keeping it focused.
  await page.click('#more');
  await expect(page.locator('#outcome')).toContainText('Save or restore');
  await expect(name).toHaveValue('Unsent later-page name');
  await expect(name).toBeFocused();
  await captureState(
    page,
    'settings-pagination-draft-guard',
    'Real bounded25-device page; unsent draft blocks navigation without discarding focus',
  );
  await page.click('#first');
  await expect(name).toHaveValue('Unsent later-page name');
  await expect(name).toBeFocused();
  await expect(page.locator('.device')).toHaveCount(25);
  // Explicit restoration permits bounded navigation; return still uses server metadata.
  await name.fill('Device 26');
  await page.click('#first');
  await expect(page.locator('#name-00000000-0000-4000-8000-000000000001')).toBeVisible();
  await page.click('#more');
  await expect(name).toHaveValue('Device 26');
  await expect(page.locator('.device')).toHaveCount(25);
  // Real non-self mutations exercise the presentation without changing the browser's authority.
  await page.unroute('**/append');
  await page.click('#first');
  const target = page
    .locator('.device')
    .filter({ has: page.locator('#name-00000000-0000-4000-8000-000000000001') });
  await target.locator('input').fill('Reviewed device');
  await target.locator('button[type=submit]').click();
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'committed');
  await expect(target.locator('.device-summary')).toContainText('Reviewed device');
  await expect(page.locator('#original')).toBeEmpty();
  await expect(page.locator('#original')).toBeHidden();
  await page.click('#more');
  await expect(page.locator('[data-feedback="shared"] [data-outcome-slot]')).toHaveText(
    'Device renamed.',
  );
  await expect(page.locator('[data-outcome-slot]').filter({ hasText: /.+/ })).toHaveCount(1);
  await expect(page.locator('#original')).toBeEmpty();
  await expect(page.locator('#original')).toBeHidden();
  await page.click('#first');
  await expect(target.locator('[data-outcome-slot]')).toHaveText('Device renamed.');
  await expect(page.locator('[data-feedback="shared"] [data-outcome-slot]')).toBeEmpty();
  await expect(page.locator('#original')).toBeEmpty();
  await expect(page.locator('#original')).toBeHidden();
  await captureState(
    page,
    'settings-other-device-renamed',
    'Committed non-self rename; unchanged current browser authority',
  );
  page.once('dialog', async (dialog) => {
    expect(dialog.type()).toBe('confirm');
    expect(dialog.message()).toBe('Revoke Reviewed device?');
    await dialog.accept();
  });
  await captureState(
    page,
    'settings-revoke-ready',
    'Native revoke action; confirm text is asserted separately, not rasterized by headless Chromium',
  );
  await target.getByRole('button', { name: 'Revoke Reviewed device', exact: true }).click();
  await expect(target.locator('.device-summary')).toContainText('Revoked');
  await expect(
    target.getByRole('button', { name: 'Revoke Reviewed device', exact: true }),
  ).toBeDisabled();
  await expect(
    target.getByRole('button', { name: 'Revoke Reviewed device', exact: true }),
  ).toHaveAccessibleDescription('This device is revoked.');
  await captureState(
    page,
    'settings-other-device-revoked',
    'Revoked row stays read-only with an explicit associated disabled reason',
  );
  // Saturate the actual retained-identity quota in this disposable Store only.
  execFileSync('python3', [
    '-c',
    "import sqlite3,sys,uuid,json;d=sqlite3.connect(sys.argv[1]);client=sys.argv[2];n=d.execute('select count(*) from management_receipts where client_id=?',(client,)).fetchone()[0];d.executemany('insert into management_receipts values(?,?,?,?,?,?,?,?)',[(str(uuid.uuid4()),client,'remote.settings.set',bytes(32),1,0,0,json.dumps({'state':'unknown','reason':'effect_outcome_unconfirmed'})) for _ in range(1000-n)]);d.commit();d.close()",
    join(root, 'state/remote/remote.db'),
    inventory.devices[0]!.clientId,
  ]);
  await page.selectOption('#opening', 'off');
  await page.click('#opening-form button');
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'refused');
  await expect(page.locator('#controls-reason')).toHaveText(
    'Browser change limit reached. Use the local CLI; do not retry or reset storage.',
  );
  await expect(page.locator('#opening-form button')).toBeDisabled();
  await captureState(
    page,
    'settings-capacity-refused',
    'Actual signed cumulative-capacity refusal; local CLI, no retry/reset/new-ID workaround',
  );
  // Malformed file read stays observational and reports default source plus warning.
  await writeFile(join(root, 'state/remote/settings.json'), '{');
  await page.click('#refresh');
  await expect(page.locator('#warning')).toHaveText(
    'settings.json could not be read; defaults apply',
  );
  await expect(page.locator('#limit-value')).toHaveText('8 · default');
  await captureState(
    page,
    'settings-malformed-warning',
    'Actual unreadable settings projection; default values/source, warning, previous refusal retained',
  );
  await context.close();
});

test('committed signed management survives owned serve SIGKILL and fresh original-ID read', async () => {
  pair = spawn(BINARY, ['pair', '--json'], { env });
  const events = lines(pair);
  const offer = await events.next();
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(offer.link as string);
  await page.fill('#name', 'Crash browser');
  await page.click('button');
  await expect(page.locator('#words')).toBeVisible();
  await events.next();
  pair.stdin.write('confirm\n');
  expect((await events.next()).reason).toBe('paired');
  await expect(page.locator('#status')).toContainText('This browser is paired.');
  await exited(pair);
  const inventory = JSON.parse(
    execFileSync(BINARY, ['devices', '--json'], { env, encoding: 'utf8' }),
  ) as { devices: { clientId: string }[] };
  execFileSync(BINARY, ['devices', 'designate', inventory.devices[0]!.clientId, '--json'], { env });
  await page.goto(`${origin}/settings`);
  await expect(page.locator('#opening')).toBeEnabled();
  const startupCalls = (await readFile(join(root, 'core-calls.jsonl'), 'utf8'))
    .split('\n')
    .filter(Boolean).length;
  let effectCalls = 0;
  let opens = 0;
  let originalId = '';
  let durable!: Record<string, unknown>;
  const beforeMount = await page.evaluate(() =>
    fetch('/sdk/mount', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: location.pathname }),
    }).then((reply) => reply.json()),
  );
  await page.route('**/append', async (route) => {
    const body = route.request().postDataJSON() as { operation: string; id: string };
    if (body.operation === 'session.open') opens++;
    if (body.operation === 'remote.settings.set') {
      effectCalls++;
      originalId = body.id;
      const reply = await route.fetch();
      expect(reply.status()).toBe(200);
      // This real effect+settlement and native response is the deterministic
      // milestone. The browser still has not received its acknowledgement.
      const response = await reply.json();
      expect(JSON.parse(Buffer.from(response.payload, 'base64url').toString('utf8')).state).toBe(
        'committed',
      );
      durable = JSON.parse(
        execFileSync(
          'python3',
          [
            '-c',
            "import sqlite3,sys,json; db=sqlite3.connect(sys.argv[1]); row=db.execute('SELECT id,adopted_ms,deadline_ms,outcome FROM management_receipts WHERE id=?',(sys.argv[2],)).fetchone(); print(json.dumps(dict(zip(['id','adopted','deadline','outcome'],row)))); db.close()",
            join(root, 'state/remote/remote.db'),
            originalId,
          ],
          { encoding: 'utf8' },
        ),
      );
      expect(JSON.parse(durable.outcome as string).state).toBe('committed');
      expect(serve.kill('SIGKILL')).toBe(true);
      await exited(serve);
      expect(serve.signalCode).toBe('SIGKILL');
      await route.abort();
    } else await route.continue();
  });
  await page.selectOption('#opening', 'off');
  await page.click('#opening-form button');
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'unknown');
  await captureState(
    page,
    'settings-crash-unknown',
    'Actual joined serve SIGKILL with original unknown; no resubmission',
  );
  await expect(page.locator('#original')).toHaveText(`Original operation ${originalId}`);
  // The killed task is joined above. Only this disposable root/owned serve restarts.
  serve = spawn(BINARY, ['serve', '--json'], { env });
  const restarted = await lines(serve).next();
  expect(new URL(restarted.address as string).origin).toBe(origin);
  const afterMount = await page.evaluate(() =>
    fetch('/sdk/mount', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: location.pathname }),
    }).then((reply) => reply.json()),
  );
  expect(afterMount.machineId).toBe(beforeMount.machineId);
  expect(afterMount.windowId).not.toBe(beforeMount.windowId);
  await page.click('#recover');
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'committed');
  await expect(page.locator('#opening-value')).toHaveText('Off · settings.json');
  await expect(page.locator('#opening')).toBeEnabled(); // designation survived process death.
  await expect(page.locator('#original')).toBeEmpty();
  await expect(page.locator('#original')).toBeHidden();
  expect(effectCalls).toBe(1);
  expect(opens).toBe(1);
  const after = JSON.parse(
    execFileSync(
      'python3',
      [
        '-c',
        "import sqlite3,sys,json; db=sqlite3.connect(sys.argv[1]); row=db.execute('SELECT id,adopted_ms,deadline_ms,outcome FROM management_receipts WHERE id=?',(sys.argv[2],)).fetchone(); print(json.dumps(dict(zip(['id','adopted','deadline','outcome'],row)))); db.close()",
        join(root, 'state/remote/remote.db'),
        originalId,
      ],
      { encoding: 'utf8' },
    ),
  );
  expect(after).toEqual(durable);
  const calls = (await readFile(join(root, 'core-calls.jsonl'), 'utf8'))
    .split('\n')
    .filter(Boolean)
    .map((line) => JSON.parse(line) as { operation: string });
  expect(
    calls
      .slice(startupCalls)
      .map((call) => call.operation)
      .sort(),
  ).toEqual(['capabilities', 'storage.root']);
  expect(calls.filter((call) => call.operation === 'dispatch.create')).toHaveLength(0);
  await context.close();
});

test('settings shared presentation keeps unavailable initialization read-only', async () => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${origin}/settings`);
  await expect(page.locator('#access')).toContainText('unconfirmed');
  await expect(page.locator('#opening')).toBeDisabled();
  await expect(page.locator('#refresh')).toBeDisabled();
  await expect(page.locator('#controls-reason')).toBeVisible();
  await captureState(
    page,
    'settings-unavailable-initialization',
    'No saved pairing; access unconfirmed, native controls remain unavailable',
  );
  await context.close();
});

/** Exactly the UX-approved Firestore matrix: ten state shots and one focus shot. */
async function captureFirestore(
  page: Page,
  state: string,
  width: number,
  theme: 'light' | 'dark',
  evidence: string,
): Promise<void> {
  await page.emulateMedia({ colorScheme: theme });
  await page.setViewportSize({ width, height: 900 });
  if (width === 390 && (state === 'observable-enabled-fixture' || state === 'partial-record')) {
    await page.locator('#firestore-budget').evaluate((element) => {
      (element as HTMLDetailsElement).open = true;
    });
  }
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  const directory = process.env.TMT_REMOTE_CAPTURE_DIR;
  if (!directory) return;
  await mkdir(directory, { recursive: true });
  const path = join(directory, `firestore-${state}-${width}-${theme}.png`);
  await page.evaluate(() => scrollTo(0, 0));
  await page.screenshot({ path, fullPage: true });
  await appendFile(
    join(directory, 'index.jsonl'),
    JSON.stringify({
      state,
      viewport: `${width}x900`,
      theme,
      path,
      evidence,
      lookAt:
        'Read-only recorded readiness, unsupported-layer one-liners; dated budget is not actual usage',
    }) + '\n',
  );
}
test('Firestore settings show recorded prerequisites and the approved fixture capture matrix without effects', async () => {
  pair = spawn(BINARY, ['pair', '--json'], { env });
  const events = lines(pair);
  const offer = await events.next();
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(offer.link as string);
  await page.fill('#name', 'Firestore browser');
  await page.click('button');
  await expect(page.locator('#words')).toBeVisible();
  await events.next();
  pair.stdin.write('confirm\n');
  expect((await events.next()).reason).toBe('paired');
  await expect(page.locator('#status')).toContainText('This browser is paired.');
  await exited(pair);
  const observed: string[] = [];
  page.on('request', (request) => {
    if (request.method() === 'POST' && new URL(request.url()).pathname.endsWith('/append'))
      observed.push(JSON.parse(request.postData()!).operation);
  });
  async function recorded(): Promise<{ firestoreLayers: unknown; firestoreBudget: unknown }> {
    return {
      firestoreLayers: JSON.parse(
        execFileSync(BINARY, ['status', '--layers', '--json'], { env, encoding: 'utf8' }),
      ).firestoreLayers,
      firestoreBudget: JSON.parse(
        execFileSync(BINARY, ['status', '--budget', '--json'], { env, encoding: 'utf8' }),
      ).firestoreBudget,
    };
  }
  // Hold the optional signed reply, not the base management reads or page assets.
  const inventory = JSON.parse(
    execFileSync(BINARY, ['devices', '--json'], { env, encoding: 'utf8' }),
  );
  execFileSync(BINARY, ['devices', 'designate', inventory.devices[0].clientId, '--json'], { env });
  const optional = routeBarrier();
  const optionalJoined = routeBarrier();
  const holdOptional = async (route: import('@playwright/test').Route) => {
    const wire = route.request().postDataJSON();
    const input = JSON.parse(Buffer.from(wire.payload, 'base64url').toString('utf8'));
    if (wire.operation !== 'remote.settings.show' || input.firestore !== true) {
      await route.continue();
      return;
    }
    const response = await route.fetch();
    optional.arrive();
    try {
      await optional.held;
      await route.fulfill({ response });
    } finally {
      optionalJoined.arrive();
    }
  };
  await page.route('**/append', holdOptional);
  try {
    await page.goto(`${origin}/settings`);
    await optional.reached;
    await expect(page.locator('#access')).toHaveText('Current browser access confirmed.');
    await expect(page.locator('#opening')).toBeEnabled();
    await expect(page.locator('#refresh')).toBeEnabled();
    await expect(page.locator('.device-summary')).toContainText('Firestore browser');
    await expect(page.locator('#firestore-content')).toHaveText(
      'Checking recorded Firestore setup…',
    );
  } finally {
    optional.release();
    await optionalJoined.reached;
    await page.unroute('**/append', holdOptional);
  }
  const failOptional = async (route: import('@playwright/test').Route) => {
    const wire = route.request().postDataJSON();
    const input = JSON.parse(Buffer.from(wire.payload, 'base64url').toString('utf8'));
    if (wire.operation === 'remote.settings.show' && input.firestore === true) {
      // Admit the read before losing its reply, preserving the signed sequence.
      const response = await route.fetch();
      await route.fulfill({ response, status: 502, body: 'optional fixture unavailable' });
    } else await route.continue();
  };
  await expect(page.locator('#firestore-content')).toContainText('Firestore is not configured.');
  await page.route('**/append', failOptional);
  try {
    await page.click('#refresh');
    await expect(page.locator('#firestore-content')).toContainText(
      'Firestore setup could not be confirmed.',
    );
    await expect(page.locator('#access')).toHaveText('Current browser access confirmed.');
    await expect(page.locator('#opening')).toBeEnabled();
    await expect(page.locator('#refresh')).toBeEnabled();
    await expect(page.locator('.device-summary')).toContainText('Firestore browser');
  } finally {
    await page.unroute('**/append', failOptional);
  }
  execFileSync(BINARY, ['devices', 'undesignate', '--json'], { env });
  await page.click('#refresh');
  await expect(page.locator('#firestore-content')).toContainText('Firestore is not configured.');
  expect((await recorded()).firestoreLayers).toEqual([]);
  await captureFirestore(
    page,
    'not-configured',
    1440,
    'light',
    'real binary; no deployment record',
  );
  await captureFirestore(page, 'not-configured', 390, 'light', 'real binary; no deployment record');

  // Visual fixture only: the native signed/control matrix and SDK validator tests own policy proof.
  // Production records cannot establish tier/quota; never label this as provider evidence.
  const enabled = [
    {
      layer: 'sharing',
      state: 'enabled',
      prerequisites: ['project', 'sign-in', 'rules', 'plan-tier', 'quota'].map((item) => ({
        item,
        state: 'enabled',
      })),
    },
    {
      layer: 'operations',
      state: 'not-enabled',
      prerequisites: [
        { item: 'plan-tier', state: 'enabled' },
        { item: 'support', state: 'not-enabled', reason: 'not-implemented' },
      ],
    },
    {
      layer: 'attachments',
      state: 'not-enabled',
      prerequisites: [
        { item: 'support', state: 'not-enabled', reason: 'not-implemented' },
        ...['project', 'rules', 'quota'].map((item) => ({ item, state: 'enabled' })),
      ],
    },
  ];
  await page.route('**/sdk/firestore-fixture.js', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `import * as sdk from '/sdk/remote-v1.js'; export * from '/sdk/remote-v1.js'; export function management(session) { const original=sdk.management(session); return {...original, settings:async(options)=>{const view=await original.settings(options);return options?{...view,firestoreLayers:${JSON.stringify(enabled)}}:view;}}; }`,
    }),
  );
  const fixtureModule = async (route: import('@playwright/test').Route) => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      body: (await response.text()).replaceAll('/sdk/remote-v1.js', '/sdk/firestore-fixture.js'),
    });
  };
  await page.route('**/sdk/settings-v1.js', fixtureModule);
  await page.goto(`${origin}/settings`);
  await expect(page.locator('[data-layer="sharing"]')).toContainText('Enabled');
  for (const layer of ['operations', 'attachments']) {
    await expect(page.locator(`[data-layer="${layer}"]`)).toContainText(
      'Not available in this release.',
    );
    expect(await page.locator(`[data-layer="${layer}"] li`).count()).toBe(0);
    expect(await page.locator(`[data-layer="${layer}"] code`).count()).toBe(0);
  }
  await captureFirestore(
    page,
    'observable-enabled-fixture',
    1440,
    'light',
    'all observable prerequisites enabled (fixture); support not implemented; served binary page with test-only SDK projection',
  );
  await captureFirestore(
    page,
    'observable-enabled-fixture',
    390,
    'light',
    'same explicit visual fixture; published budget is real binary status --budget',
  );
  await captureFirestore(
    page,
    'observable-enabled-fixture',
    1440,
    'dark',
    'same explicit visual fixture; no provider evidence',
  );
  await page.unroute('**/sdk/settings-v1.js', fixtureModule);
  await page.unroute('**/sdk/firestore-fixture.js');

  const partial = {
    version: 1,
    record: {
      deploymentId: '3f2b8c1e-5d4a-4e7b-9c1d-2a6f8e0b4c11',
      binding: null,
      run: {
        planDigest: 'a'.repeat(64),
        account: 'fixture@example.test',
        authorizedAtMs: 1,
        state: 'partial',
        rulesAttempted: true,
        steps: [
          { id: 'database', state: 'done' },
          { id: 'sign-in:anonymous', state: 'unknown' },
          { id: 'rules', state: 'unknown' },
          { id: 'verify', state: 'pending' },
        ],
      },
    },
  };
  await writeFile(join(root, 'state/remote/deploy.json'), JSON.stringify(partial), { mode: 0o600 });
  await page.goto(`${origin}/settings`);
  await expect(page.locator('[data-item="rules"]')).toContainText(
    'Firestore rules are only partly deployed.',
  );
  await expect(page.locator('[data-item="sign-in"]')).toContainText('Not checked');
  await expect(page.locator('[data-item="rules"] code')).toHaveText('tmt remote deploy firestore');
  const actual = await recorded();
  expect((actual.firestoreLayers as { state: string }[])[0]!.state).toBe('not-enabled');
  await captureFirestore(
    page,
    'partial-record',
    1440,
    'light',
    'real binary; private partial fixture record; signed read and status agree',
  );
  await captureFirestore(
    page,
    'partial-record',
    390,
    'light',
    'same private record; budget expanded, no horizontal scroll',
  );
  await captureFirestore(
    page,
    'partial-record',
    1440,
    'dark',
    'same private fixture record, not provider currentness',
  );

  await writeFile(join(root, 'state/remote/deploy.json'), 'damaged fixture', { mode: 0o600 });
  await page.goto(`${origin}/settings`);
  await expect(page.locator('[data-layer="sharing"]')).toContainText('Not checked');
  expect((await recorded()).firestoreLayers).toEqual(
    expect.arrayContaining([expect.objectContaining({ layer: 'sharing', state: 'unknown' })]),
  );
  expect(await page.locator('[data-layer="sharing"] li').count()).toBe(5);
  for (const item of ['project', 'sign-in', 'rules', 'plan-tier', 'quota'])
    await expect(page.locator(`[data-item="${item}"]`)).toContainText('Not checked');
  await captureFirestore(
    page,
    'unknown-record',
    1440,
    'light',
    'damaged private record; all observable prerequisites not checked; support unchanged',
  );
  await captureFirestore(
    page,
    'unknown-record',
    390,
    'light',
    'same damaged private record; no repair or provider call',
  );
  const details = page.locator('#firestore-budget');
  await details.evaluate((element) => {
    (element as HTMLDetailsElement).open = false;
  });
  const summary = details.locator('summary');
  await summary.focus();
  await expect(summary).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(details).toHaveAttribute('open', '');
  await captureFirestore(
    page,
    'keyboard-focus',
    390,
    'light',
    'native Details summary focused and expanded by keyboard',
  );
  await expect(page.locator('#firestore-budget')).toContainText(
    "Remote can't see your actual usage",
  );
  await expect(page.locator('#firestore-budget')).toContainText('70%');
  await expect(page.locator('#firestore-budget')).toContainText('90%');
  expect(observed).toEqual(expect.arrayContaining(['remote.settings.show', 'remote.devices.list']));
  expect(
    // A lost read reply invokes the SDK's scope-free sequence recovery read.
    observed.every((op) =>
      ['session.open', 'capabilities', 'remote.settings.show', 'remote.devices.list'].includes(op),
    ),
    `Observed signed operations: ${JSON.stringify(observed)}`,
  ).toBe(true);
  // The damaged fixture is never repaired by inspection.
  expect(await readFile(join(root, 'state/remote/deploy.json'), 'utf8')).toBe('damaged fixture');
  await context.close();
});

/** Three populated devices over the existing private native door; no page-state mock. */
async function feedbackFixture(width: number, theme: 'light' | 'dark') {
  pair = spawn(BINARY, ['pair', '--json'], { env });
  const events = lines(pair);
  const offer = await events.next();
  const context = await browser.newContext({
    viewport: { width, height: 900 },
    colorScheme: theme,
  });
  const page = await context.newPage();
  await page.goto(offer.link as string);
  await page.fill('#name', 'Feedback browser');
  await page.click('button');
  await expect(page.locator('#words')).toBeVisible();
  await events.next();
  pair.stdin.write('confirm\n');
  expect((await events.next()).reason).toBe('paired');
  await exited(pair);
  const inventory = JSON.parse(
    execFileSync(BINARY, ['devices', '--json'], { env, encoding: 'utf8' }),
  ) as { devices: { clientId: string }[] };
  const clientId = inventory.devices[0]!.clientId;
  execFileSync(BINARY, ['devices', 'designate', clientId, '--json'], { env });
  execFileSync('python3', [
    '-c',
    "import sqlite3,sys;d=sqlite3.connect(sys.argv[1]);d.executemany('INSERT INTO grants VALUES (?,?,?,?,?,?,?,?,?,?,?,?)',[('00000000-0000-4000-8000-%012d'%i,i.to_bytes(32,'big'),'browser',sys.argv[2],'Device %d'%i,'all','capabilities','direct',0,None,1,0) for i in range(1,3)]);d.commit();d.close()",
    join(root, 'state/remote/remote.db'),
    origin,
  ]);
  await page.goto(`${origin}/settings`);
  await expect(page.locator('#access')).toContainText('confirmed');
  await expect(page.locator('.device')).toHaveCount(3);
  return { context, page, clientId };
}

for (const [width, theme] of [
  [1440, 'light'],
  [390, 'dark'],
] as const) {
  test(`settings action feedback is local and unchanged Saves stay effect-free (${width}/${theme})`, async () => {
    const { context, page, clientId } = await feedbackFixture(width, theme);
    const targetId = '00000000-0000-4000-8000-000000000001';
    const row = page.locator('.device').filter({ has: page.locator(`#name-${targetId}`) });
    const slot = row.locator('[data-outcome-slot]');
    const writes: { operation: string; input: { operationId: string } }[] = [];
    const reads: string[] = [];
    page.on('request', (request) => {
      if (!new URL(request.url()).pathname.endsWith('/append')) return;
      const wire = request.postDataJSON();
      if (
        [
          'remote.settings.set',
          'remote.devices.rename',
          'remote.devices.revoke',
          'remote.devices.talk',
        ].includes(wire.operation)
      )
        writes.push({
          operation: wire.operation,
          input: JSON.parse(Buffer.from(wire.payload, 'base64url').toString('utf8')),
        });
      if (wire.operation === 'remote.management.operation')
        reads.push(JSON.parse(Buffer.from(wire.payload, 'base64url').toString('utf8')).operationId);
    });
    // Empty polite slots exist before any action can publish text; retain node identity.
    await expect(page.locator('[data-outcome-slot]')).toHaveCount(6);
    for (const status of await page.locator('[data-outcome-slot]').all()) {
      await expect(status).toHaveAttribute('role', 'status');
      await expect(status).toHaveAttribute('aria-live', 'polite');
      await expect(status).toHaveAttribute('aria-atomic', 'true');
      await expect(status).toBeEmpty();
    }
    await slot.evaluate((status) => {
      const fixture = window as typeof window & {
        feedbackSlot?: Element;
        duplicateOutcomes?: boolean;
      };
      fixture.feedbackSlot = status;
      fixture.duplicateOutcomes = false;
      new MutationObserver(() => {
        const populated = [...document.querySelectorAll('[data-outcome-slot]')].filter(
          (item) => item.textContent,
        );
        if (populated.length > 1) fixture.duplicateOutcomes = true;
      }).observe(document.querySelector('main')!, { childList: true, subtree: true });
    });
    const openingSave = page.locator('#opening-form button[type=submit]');
    const limitSave = page.locator('#limit-form button[type=submit]');
    await expect(openingSave).toBeDisabled();
    await expect(limitSave).toBeDisabled();
    await expect(openingSave).toHaveAccessibleDescription('No changes to save.');
    await expect(limitSave).toHaveAccessibleDescription('No changes to save.');
    // Align the originating form before its action, never scroll to manufacture feedback visibility.
    await page
      .locator('#opening-form')
      .evaluate((form) =>
        scrollTo(0, form.closest('section')!.getBoundingClientRect().top + scrollY - 70),
      );
    expect(
      await page.evaluate(() => {
        const original = crypto.randomUUID;
        let allocations = 0;
        crypto.randomUUID = () => {
          allocations++;
          return original.call(crypto);
        };
        try {
          (document.querySelector('#opening-form') as HTMLFormElement).requestSubmit();
          (document.querySelector('#limit-form') as HTMLFormElement).requestSubmit();
        } finally {
          crypto.randomUUID = original;
        }
        return allocations;
      }),
    ).toBe(0);
    expect(writes).toHaveLength(0);
    await captureState(
      page,
      'feedback-unchanged-save',
      'Disabled unchanged browser-opening Save with its described hint; default cap stays unset',
      [width],
      [theme],
      '#opening-unchanged',
    );
    await page.selectOption('#opening', 'off');
    await expect(openingSave).toBeEnabled();
    await expect(page.locator('#opening-unchanged')).toBeHidden();
    await page.selectOption('#opening', 'on');
    await expect(openingSave).toBeDisabled();
    await page
      .locator('#limit-form')
      .evaluate((form) =>
        scrollTo(0, form.closest('section')!.getBoundingClientRect().top + scrollY - 70),
      );
    await page.locator('#limit-form').evaluate((form) => (form as HTMLFormElement).requestSubmit());
    await feedbackBounds(page, '#limit-unchanged');
    await page.selectOption('#limit-mode', 'custom');
    await page.fill('#limit-custom', '');
    await expect(limitSave).toBeDisabled();
    await page.fill('#limit-custom', '8');
    await expect(limitSave).toBeEnabled(); // Explicit 8 differs from admitted default provenance.
    await page.locator('#limit-mode').evaluate((select) => {
      (select as HTMLSelectElement).value = 'default';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await expect(limitSave).toBeDisabled();
    expect(writes).toHaveLength(0);
    expect(
      JSON.parse(execFileSync(BINARY, ['settings', '--json'], { env, encoding: 'utf8' }))
        .sessionsPerDeviceSource,
    ).toBe('default');

    const name = row.locator('input');
    await name.fill('Renamed device');
    await row.evaluate((form) => scrollTo(0, form.getBoundingClientRect().top + scrollY - 70));
    await name.press('Enter');
    await expect(slot).toHaveText('Device renamed.');
    await expect(slot).toHaveAttribute('data-state', 'committed');
    await expect(row.locator('[data-original]')).toBeEmpty();
    await expect(row.locator('[data-original]')).toBeHidden();
    await expect(name).toBeFocused();
    await expect(row.locator('.device-summary')).toContainText('Renamed device');
    expect(
      await slot.evaluate(
        (status) => status === (window as typeof window & { feedbackSlot?: Element }).feedbackSlot,
      ),
    ).toBe(true);
    await expect(page.locator('[data-outcome-slot]').filter({ hasText: /.+/ })).toHaveCount(1);
    await captureState(
      page,
      'feedback-committed-rename',
      'Committed original beside its UUID-bound form; focus retained, no post-action scroll',
      [width],
      [theme],
      `[data-feedback="${targetId}"] [data-outcome-slot]`,
    );
    expect(writes).toHaveLength(1);
    // A changed setting draft only clears its hint, never this retained device outcome.
    await page.selectOption('#opening', 'off');
    await expect(slot).toHaveText('Device renamed.');
    await page.selectOption('#opening', 'on');
    await expect(openingSave).toBeDisabled();

    execFileSync(BINARY, ['devices', 'undesignate', '--json'], { env });
    await name.fill('Refused draft');
    await row.evaluate((form) => scrollTo(0, form.getBoundingClientRect().top + scrollY - 70));
    await name.press('Enter');
    await expect(slot).toHaveText('This browser is read-only. Use the local CLI to make changes.');
    await expect(slot).toHaveAttribute('data-state', 'refused');
    await expect(name).toHaveValue('Refused draft');
    await captureState(
      page,
      'feedback-refused-action',
      'Signed refusal remains beside the original form; current access is separate',
      [width],
      [theme],
      `[data-feedback="${targetId}"] [data-outcome-slot]`,
    );
    execFileSync(BINARY, ['devices', 'designate', clientId, '--json'], { env });
    await page.click('#refresh');
    await expect(name).toBeEnabled();
    const published = routeBarrier();
    const joined = routeBarrier();
    const lostAck = async (route: import('@playwright/test').Route) => {
      const wire = route.request().postDataJSON();
      if (wire.operation !== 'remote.devices.rename') {
        await route.continue();
        return;
      }
      try {
        expect((await route.fetch()).status()).toBe(200);
        published.arrive();
        await published.held;
        await route.abort();
      } finally {
        joined.arrive();
      }
    };
    await page.route('**/append', lostAck);
    await name.fill('Recovered device');
    await row.evaluate((form) => scrollTo(0, form.getBoundingClientRect().top + scrollY - 70));
    await name.press('Enter');
    await published.reached;
    try {
      await expect(slot).toHaveText('Saving…');
      await expect(name).toBeFocused();
      await name.fill('Unsent next draft');
      await name.evaluate((input) => (input as HTMLInputElement).setSelectionRange(2, 2));
    } finally {
      published.release();
    }
    await joined.reached;
    await expect(slot).toHaveText(
      'This change could not be confirmed. Check its original result before making another change.',
    );
    await expect(slot).toHaveAttribute('data-state', 'unknown');
    await expect(name).toBeFocused();
    await expect(name).toHaveValue('Unsent next draft');
    const originalId = writes.at(-1)!.input.operationId;
    await expect(page.locator('#original')).toHaveText(`Original operation ${originalId}`);
    await expect(row.locator(`#device-reason-${targetId}`)).toBeEmpty();
    await expect(row.locator(`#device-reason-${targetId}`)).toBeHidden();
    for (const otherId of [clientId, '00000000-0000-4000-8000-000000000002'])
      await expect(page.locator(`#device-reason-${otherId}`)).toHaveText(
        'Another change is still unconfirmed. Check its original result first.',
      );
    await expect(
      row.getByRole('button', { name: 'Check original result', exact: true }),
    ).toBeVisible();
    await captureState(
      page,
      'feedback-unknown-original',
      'Lost native acknowledgement; recovery beside the unknown original, no resend',
      [width],
      [theme],
      `[data-feedback="${targetId}"] [data-outcome-slot], [data-feedback="${targetId}"] [data-recover]`,
    );
    await page.unroute('**/append', lostAck);
    await name.press('Enter');
    expect(writes).toHaveLength(3);
    await page.click('#recover');
    await expect(slot).toHaveText('Device renamed.');
    expect(reads).toEqual([originalId]);
    expect(writes).toHaveLength(3);
    await expect(name).toHaveValue('Unsent next draft');
    expect(await name.evaluate((input) => (input as HTMLInputElement).selectionStart)).toBe(2);
    expect(
      await page.evaluate(
        () => (window as typeof window & { duplicateOutcomes?: boolean }).duplicateOutcomes,
      ),
    ).toBe(false);
    await context.close();
  });
}

for (const [width, theme] of [
  [1440, 'light'],
  [390, 'dark'],
] as const) {
  test(`settings accessibility preserves refresh focus and quiet projections (${width}/${theme})`, async () => {
    const { context, page, clientId } = await feedbackFixture(width, theme);
    await expect(page.locator('#firestore-budget')).toBeVisible();
    await page.evaluate(() => {
      const state = window as typeof window & { a11yTrace?: string[] };
      state.a11yTrace = [];
      for (const kind of ['focusin', 'focusout'])
        document.addEventListener(kind, (event) => {
          const target = event.target as HTMLElement;
          state.a11yTrace!.push(`${kind}:${target.id || target.tagName}`);
        });
    });
    async function gatedRefresh(move: boolean): Promise<void> {
      const barrier = routeBarrier();
      const joined = routeBarrier();
      const hold = async (route: import('@playwright/test').Route) => {
        const wire = route.request().postDataJSON();
        const input = JSON.parse(Buffer.from(wire.payload, 'base64url').toString('utf8'));
        if (wire.operation !== 'remote.settings.show' || input.firestore === true) {
          await route.continue();
          return;
        }
        barrier.arrive();
        try {
          await barrier.held;
          await route.continue();
        } finally {
          joined.arrive();
        }
      };
      await page.route('**/append', hold);
      await page.locator('#refresh').focus();
      await page.locator('#refresh').press('Enter');
      try {
        await barrier.reached;
        await expect(page.locator('#refresh')).toBeDisabled();
        if (move) {
          // A real intentional move outside the disabled form must never be stolen back.
          await page.evaluate(() => {
            const probe = document.createElement('button');
            probe.id = 'focus-probe';
            probe.textContent = 'Fixture focus target';
            document.body.append(probe);
            probe.focus();
          });
        }
      } finally {
        barrier.release();
        await joined.reached;
        await page.unroute('**/append', hold);
      }
      await expect(page.locator('#refresh')).toBeEnabled();
      await expect(page.locator(move ? '#focus-probe' : '#refresh')).toBeFocused();
      if (move) await page.locator('#focus-probe').evaluate((probe) => probe.remove());
      await expect(page.locator('#firestore-budget')).toBeVisible();
    }
    await gatedRefresh(false);
    await gatedRefresh(true);
    const own = page.locator(`#name-${clientId}`);
    await expect(own).toHaveAccessibleName('Device name for Feedback browser');
    await expect(
      page.getByRole('button', { name: 'Rename this device (Feedback browser)', exact: true }),
    ).toBeEnabled();
    const other = page.locator('#name-00000000-0000-4000-8000-000000000001');
    await expect(other).toHaveAccessibleName('Device name for Device 1');
    await expect(page.getByRole('button', { name: 'Rename Device 1', exact: true })).toBeEnabled();
    await expect(
      page.getByRole('button', { name: 'Enable sending for Device 1', exact: true }),
    ).toBeEnabled();
    await expect(page.getByRole('button', { name: 'Revoke Device 1', exact: true })).toBeEnabled();
    await page.locator('#firestore-budget summary').click();
    await page.evaluate(() => {
      const state = window as typeof window & { quiet?: { count: number; budget: Element } };
      state.quiet = { count: 0, budget: document.querySelector('#firestore-budget')! };
      new MutationObserver((records) => {
        state.quiet!.count += records.length;
      }).observe(document.querySelector('#access-announcement')!, {
        childList: true,
        characterData: true,
        subtree: true,
      });
      new MutationObserver((records) => {
        state.quiet!.count += records.length;
      }).observe(document.querySelector('#firestore-content')!, {
        childList: true,
        characterData: true,
        subtree: true,
      });
    });
    await page.selectOption('#limit-mode', 'custom');
    await page.locator('#limit-custom').fill('');
    await page.locator('#limit-custom').pressSequentially('123');
    await page.locator('#limit-custom').press('Tab');
    expect(
      await page.evaluate(() => {
        const state = window as typeof window & { quiet?: { count: number; budget: Element } };
        return {
          count: state.quiet!.count,
          same: state.quiet!.budget === document.querySelector('#firestore-budget'),
          open: document.querySelector<HTMLDetailsElement>('#firestore-budget')!.open,
        };
      }),
    ).toEqual({ count: 0, same: true, open: true });
    await page.locator('#refresh').focus();
    await captureState(
      page,
      'a11y-settings-refreshed',
      'Refresh focus and admitted device action names; budget outside live region',
      [width],
      [theme],
    );
    execFileSync(BINARY, ['devices', 'undesignate', '--json'], { env });
    await gatedRefresh(false);
    await expect(page.locator('#read-only')).toBeVisible();
    const directory = process.env.TMT_REMOTE_CAPTURE_DIR;
    if (directory)
      await appendFile(
        join(directory, 'focus-traces.jsonl'),
        JSON.stringify({
          state: 'settings',
          width,
          theme,
          trace: await page.evaluate(
            () => (window as typeof window & { a11yTrace?: string[] }).a11yTrace,
          ),
        }) + '\n',
      );
    await context.close();
  });

  test(`pairing accessibility follows visible ceremony focus (${width}/${theme})`, async () => {
    pair = spawn(BINARY, ['pair', '--json'], { env });
    const events = lines(pair);
    const offer = await events.next();
    const context = await browser.newContext({
      viewport: { width, height: 900 },
      colorScheme: theme,
    });
    const page = await context.newPage();
    await page.goto(offer.link as string);
    await page.evaluate(() => {
      const state = window as typeof window & { a11yTrace?: string[] };
      state.a11yTrace = [];
      for (const kind of ['focusin', 'focusout'])
        document.addEventListener(kind, (event) => {
          const target = event.target as HTMLElement;
          state.a11yTrace!.push(`${kind}:${target.id || target.tagName}`);
        });
    });
    await page.fill('#name', 'Keyboard browser');
    await page.locator('#pair button').focus();
    await page.locator('#pair button').press('Enter');
    await expect(page.locator('#comparison')).toBeVisible();
    await expect(page.locator('#comparison')).toBeFocused();
    await expect(page.locator('#comparison')).toHaveAccessibleName('Words to compare');
    await captureState(
      page,
      'a11y-pair-waiting',
      'Visible named word-comparison focus while terminal confirmation is held',
      [width],
      [theme],
    );
    await events.next();
    pair.stdin.write('confirm\n');
    expect((await events.next()).reason).toBe('paired');
    await exited(pair);
    await expect(page.locator('#entry-link a')).toBeFocused();
    await expect(page.locator('#status')).toContainText('This browser is paired.');
    await captureState(
      page,
      'a11y-pair-paired',
      'Return to Remote owns focus only after the same ceremony transition',
      [width],
      [theme],
    );
    const directory = process.env.TMT_REMOTE_CAPTURE_DIR;
    if (directory)
      await appendFile(
        join(directory, 'focus-traces.jsonl'),
        JSON.stringify({
          state: 'pairing',
          width,
          theme,
          trace: await page.evaluate(
            () => (window as typeof window & { a11yTrace?: string[] }).a11yTrace,
          ),
        }) + '\n',
      );
    await context.close();
    // A subsequent ceremony must not steal intentional focus on another control.
    pair = spawn(BINARY, ['pair', '--json'], { env });
    const secondEvents = lines(pair);
    const secondOffer = await secondEvents.next();
    const movedContext = await browser.newContext();
    const movedPage = await movedContext.newPage();
    await movedPage.goto(secondOffer.link as string);
    await movedPage.fill('#name', 'Moved focus browser');
    await movedPage.locator('#pair button').focus();
    await movedPage.locator('#pair button').press('Enter');
    await expect(movedPage.locator('#comparison')).toBeFocused();
    await secondEvents.next();
    await movedPage.evaluate(() => {
      const probe = document.createElement('button');
      probe.id = 'focus-probe';
      probe.textContent = 'Fixture focus target';
      document.body.append(probe);
      probe.focus();
    });
    pair.stdin.write('confirm\n');
    expect((await secondEvents.next()).reason).toBe('paired');
    await exited(pair);
    await expect(movedPage.locator('#entry-link')).toBeVisible();
    await expect(movedPage.locator('#focus-probe')).toBeFocused();
    await movedContext.close();
  });
}
