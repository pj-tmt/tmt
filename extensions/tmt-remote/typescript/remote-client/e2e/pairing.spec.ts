import { expect, test, chromium, type Browser, type Page } from '@playwright/test';
import { spawn, execFileSync, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { createHash, createPublicKey, verify } from 'node:crypto';
import { extCertSigningBytes } from '../src/canonical-bytes.js';
import type { ExtCertificate } from '../src/device.js';
import {
  appendFile,
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from 'node:fs/promises';
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
  directory = process.env.TMT_REMOTE_CAPTURE_DIR,
  phase?: Record<string, unknown>,
): Promise<void> {
  const snapshot = await page.evaluate(() => ({
    theme: matchMedia('(prefers-color-scheme: dark)').matches
      ? ('dark' as const)
      : ('light' as const),
    viewport: { width: innerWidth, height: innerHeight },
    refresh: (() => {
      const button = document.querySelector<HTMLButtonElement>('#refresh');
      return button
        ? { disabled: button.disabled, ariaBusy: button.getAttribute('aria-busy') }
        : null;
    })(),
    readOnly: (() => {
      const note = document.querySelector<HTMLElement>('#read-only');
      return note ? { hidden: note.hidden, text: note.textContent?.trim() } : null;
    })(),
    outcome: document.querySelector('#outcome')?.getAttribute('data-state') ?? null,
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
          ariaLabel: button.getAttribute('aria-label'),
          labelledBy: button
            .getAttribute('aria-labelledby')
            ?.split(/\s+/)
            .map((id) => document.getElementById(id)?.textContent?.trim()),
          disabled: button.matches(':disabled'),
          disabledProperty: button.disabled,
          disabledAttribute: button.getAttribute('disabled'),
          ariaBusy: button.getAttribute('aria-busy'),
          connected: button.isConnected,
          hidden: button.hidden,
          display: style.display,
          visibility: style.visibility,
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
  const expected = {
    color: tokenRgb('color', 'disabled-text', snapshot.theme),
    background: tokenRgb('surface', 'disabled', snapshot.theme),
    opacity: '1',
    shadow: 'none',
  };
  const { controls, ...presentation } = snapshot;
  for (const control of controls) {
    const diagnostic = `${state}: ${JSON.stringify({ head: SOURCE_HEAD, phase, ...presentation, control, expected })}`;
    expect(control.context, diagnostic).toMatchObject({
      disabled: true,
      disabledProperty: true,
      connected: true,
    });
    expect(control.style, diagnostic).toEqual(expected);
  }
  if (contained) await feedbackBounds(page, contained);
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
const SOURCE_HEAD = execFileSync('git', ['rev-parse', 'HEAD'], {
  cwd: fileURLToPath(new URL('../', import.meta.url)),
  encoding: 'utf8',
}).trim();

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
    connection.on('error', (error: NodeJS.ErrnoException) => {
      // Navigation may close the peer before the fixture finishes its response.
      if (connection.destroyed && (error.code === 'EPIPE' || error.code === 'ECONNRESET')) return;
      throw error;
    });
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

test('fixture Colab ignores only closed-peer pipe/reset errors and surfaces other failures', async () => {
  const accepted = new Promise<Socket>((resolve) => fixture.once('connection', resolve));
  const client = createConnection(join(root, 'state/colab/door.sock'));
  const connection = await accepted;
  const pipe = Object.assign(new Error('Fixture closed-peer pipe'), { code: 'EPIPE' });
  const reset = Object.assign(new Error('Fixture closed-peer reset'), { code: 'ECONNRESET' });
  const other = Object.assign(new Error('Fixture unexpected socket error'), { code: 'EIO' });
  try {
    expect(connection.destroyed).toBe(false);
    for (const error of [pipe, reset, other])
      expect(() => connection.emit('error', error)).toThrow(error);
    const closed = new Promise<void>((resolve) => connection.once('close', () => resolve()));
    client.destroy();
    await closed;
    expect(connection.destroyed).toBe(true);
    for (const error of [pipe, reset]) expect(() => connection.emit('error', error)).not.toThrow();
    expect(() => connection.emit('error', other)).toThrow(other);
  } finally {
    client.destroy();
    connection.destroy();
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
    TMT_AGENT: '1',
    BROWSER: 'true',
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
// The one teardown owner. It releases only what this test assigned (a setup that failed early
// leaves the rest unassigned, never a prior test's), runs every step even when an earlier one
// rejects, and rethrows after the root is removed.
test.afterEach(async () => {
  const held: {
    browser?: Browser;
    pair?: ChildProcessWithoutNullStreams;
    serve?: ChildProcessWithoutNullStreams;
    fixture?: Server;
    root?: string;
  } = { browser, pair, serve, fixture, root };
  browser = undefined as never;
  pair = undefined;
  serve = undefined as never;
  fixture = undefined as never;
  root = undefined as never;
  const failures: unknown[] = [];
  const step = async (release: () => unknown) => {
    try {
      await release();
    } catch (error) {
      failures.push(error);
    }
  };
  await step(() => held.browser?.close());
  // A failed run can leave the owner's pair client waiting; end it before serve.
  for (const child of [held.pair, held.serve])
    await step(async () => {
      if (!child) return;
      child.kill('SIGTERM');
      await exited(child);
    });
  await step(() => held.fixture && new Promise((resolve) => held.fixture!.close(resolve)));
  await step(() => held.root && rm(held.root, { recursive: true, force: true }));
  if (failures.length === 1) throw failures[0];
  if (failures.length) throw new AggregateError(failures, 'Pairing teardown failed');
});

/** Test-only owner socket access; production extensions use the versioned native CLI seam. */
async function automaticOffer(target: string): Promise<{ link: string; offerId: string }> {
  return new Promise((resolve, reject) => {
    const socket = createConnection(join(root, 'state/remote/control.sock'));
    socket.once('error', reject);
    socket.once('connect', () => socket.write(`${JSON.stringify({ op: 'auto-pair', target })}\n`));
    const reader = createInterface({ input: socket });
    reader.once('line', (line) => {
      reader.close();
      socket.end();
      const value = JSON.parse(line) as { link: string; offerId: string; error?: unknown };
      if (value.error || !value.link) reject(new Error('Native auto offer was refused.'));
      else resolve(value);
    });
  });
}

async function expectAutomaticLabels(page: Page, blocked: boolean) {
  await expect(page.locator('.header-title')).toHaveText('Connect this browser');
  await expect(page.locator('#heading')).toHaveText('Connect this browser');
  await expect(page.locator('.eyebrow')).toBeHidden();
  await expect(page.locator('#state-label')).toHaveText(blocked ? 'Not connected' : 'Waiting');
}

/** Commands stay whole without overflowing the real pairing notice at either width. */
async function expectUnbrokenPairingCommands(page: Page) {
  const viewport = page.viewportSize();
  const originalTheme = await page.evaluate(() =>
    matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
  );
  try {
    for (const theme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: theme });
      for (const width of [390, 1280]) {
        await page.setViewportSize({ width, height: 900 });
        const commands = await page.locator('#status code').evaluateAll((nodes) =>
          nodes.map((node) => {
            const box = node.getBoundingClientRect();
            const range = document.createRange();
            range.selectNodeContents(node);
            return {
              text: node.textContent,
              whiteSpace: getComputedStyle(node).whiteSpace,
              fragments: range.getClientRects().length,
              left: box.left,
              right: box.right,
            };
          }),
        );
        const detail = JSON.stringify({ head: SOURCE_HEAD, theme, width, commands });
        expect(commands.length, detail).toBeGreaterThan(0);
        for (const command of commands) {
          expect(command.whiteSpace, detail).toBe('nowrap');
          expect(command.fragments, detail).toBe(1);
          expect(command.left, detail).toBeGreaterThanOrEqual(0);
          expect(command.right, detail).toBeLessThanOrEqual(width);
        }
      }
    }
  } finally {
    await page.emulateMedia({ colorScheme: originalTheme });
    if (viewport) await page.setViewportSize(viewport);
  }
}

test('native plain opening preserves a manual offer and leaves an unpaired browser unpaired', async () => {
  pair = spawn(BINARY, ['pair', '--json'], { env });
  const events = lines(pair);
  const offer = await events.next();
  const page = await browser.newPage();
  const descriptor = await page.request.get(`${origin}/sdk/pair-offer`);
  const original = await descriptor.json();
  const bin = join(root, 'opener-bin');
  await mkdir(bin);
  for (const name of ['open', 'xdg-open']) await symlink('/usr/bin/printf', join(bin, name));
  const answer = JSON.parse(
    execFileSync(BINARY, ['launch', '/remote/', '--open'], {
      env: { ...env, PATH: `${bin}:${env.PATH ?? ''}` },
      encoding: 'utf8',
    }),
  ) as { version: number; link: string; opened: boolean; state: string };
  expect(answer).toEqual({ version: 1, link: `${origin}/remote/`, opened: true, state: 'opened' });
  await page.goto(answer.link);
  await expect(page.locator('#state-label')).toHaveText('Not paired');
  expect(page.url()).toBe(`${origin}/remote/`);
  expect(await (await page.request.get(`${origin}/sdk/pair-offer`)).json()).toEqual(original);
  expect(
    JSON.parse(execFileSync(BINARY, ['devices', '--json'], { env, encoding: 'utf8' })).devices,
  ).toHaveLength(0);
  expect(original).toEqual(offer.descriptor);
  // The existing teardown owner closes the still-live manual command and joins serve.
});

test('native automatic opening is single-use, strips history and reuses a paired browser', async () => {
  const page = await browser.newPage();
  const requests: { url: string; referer: string }[] = [];
  page.on('request', (request) =>
    requests.push({ url: request.url(), referer: request.headers().referer ?? '' }),
  );
  let logs = '';
  serve.stderr.on('data', (bytes: Buffer) => {
    logs += bytes.toString();
  });
  const first = await automaticOffer('/p/abcd');
  const code = new URL(first.link).hash.slice(6);
  const response = await page.goto(first.link, { waitUntil: 'domcontentloaded' });
  expect(response?.headers()['referrer-policy']).toBe('no-referrer');
  expect(response?.headers()['content-security-policy']).toContain("'sha256-");
  expect(response?.headers()['content-security-policy']).not.toContain("'unsafe-inline'");
  await page.waitForURL(`${origin}/p/abcd`);
  const list = () =>
    JSON.parse(execFileSync(BINARY, ['devices', '--json'], { env, encoding: 'utf8' })) as {
      devices: { clientId: string; origin: string; issuedAtMs: number; scopes: string[] }[];
    };
  const paired = list();
  expect(paired.devices).toHaveLength(1);
  expect(paired.devices[0]!.origin).toBe(origin);
  expect(paired.devices[0]!.issuedAtMs).toBeGreaterThan(0);
  expect(paired.devices[0]!.scopes).toEqual([
    'agents.read',
    'check.read',
    'results.read',
    'status.read',
  ]);
  expect(logs).toContain(paired.devices[0]!.clientId);
  expect(logs).toContain(origin);
  expect(logs).not.toContain(code);
  expect(
    requests.every((request) => !request.url.includes(code) && !request.referer.includes(code)),
  ).toBe(true);

  const second = await automaticOffer('/remote/');
  await page.goto(second.link, { waitUntil: 'domcontentloaded' });
  await page.waitForURL(`${origin}/remote/`);
  expect(list().devices).toHaveLength(1);
  expect(list().devices[0]!.clientId).toBe(paired.devices[0]!.clientId);
  expect((await page.request.get(`${origin}/sdk/pair-offer`)).status()).toBe(404);
  await page.goBack();
  expect(page.url()).not.toContain('#');
  await page.goForward();
  expect(page.url()).not.toContain('#');

  // A second browser/profile has no original key and cannot reuse even the exact launch link.
  const other = await browser.newContext();
  try {
    const stranger = await other.newPage();
    await stranger.goto(second.link);
    await expect(stranger.locator('#status')).toHaveText(
      'This link no longer works. Open the page again from the machine with tmt remote open.',
    );
    await expect(stranger.locator('#status code')).toHaveText('tmt remote open');
    await expectUnbrokenPairingCommands(stranger);
    expect(stranger.url()).toBe(`${origin}/remote/pair`);
    expect(list().devices).toHaveLength(1);
    await expectAutomaticLabels(stranger, true);
    await captureState(stranger, 'automatic-used', '#notice', [1280, 390], ['light', 'dark']);
    // A fresh native offer works for another profile even though the machine already has a device.
    const fresh = await automaticOffer('/read/abcd');
    await stranger.goto(fresh.link, { waitUntil: 'domcontentloaded' });
    await stranger.waitForURL(`${origin}/read/abcd`);
    expect(list().devices).toHaveLength(2);
  } finally {
    await other.close();
  }
});

test('native auto bootstrap erases before the SDK request and never shows the manual form', async () => {
  const page = await browser.newPage();
  const offer = await automaticOffer('/remote/');
  let release!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  let arrived!: () => void;
  const reached = new Promise<void>((resolve) => {
    arrived = resolve;
  });
  await page.route('**/sdk/remote-v1.js', async (route) => {
    arrived();
    await held;
    await route.continue();
  });
  const navigation = page.goto(offer.link, { waitUntil: 'domcontentloaded' });
  try {
    await reached;
    expect(page.url()).toBe(`${origin}/remote/pair`);
    expect(await page.evaluate(() => location.hash)).toBe('');
    await expect(page.locator('#pair')).toBeHidden();
    release();
    await navigation;
    await page.waitForURL(`${origin}/remote/`);
  } finally {
    release();
    await navigation.catch(() => {});
  }
});

test('native automatic connection has a code-free waiting state', async () => {
  const page = await browser.newPage();
  const offer = await automaticOffer('/remote/');
  const barrier = routeBarrier();
  await page.route('**/sdk/pair-offer', async (route) => {
    barrier.arrive();
    await barrier.held;
    await route.continue();
  });
  try {
    await page.goto(offer.link, { waitUntil: 'domcontentloaded' });
    await barrier.reached;
    expect(page.url()).toBe(`${origin}/remote/pair`);
    await expect(page.locator('#status')).toHaveText('Connecting this browser…');
    await expect(page.locator('#pair')).toBeHidden();
    await expectAutomaticLabels(page, false);
    await captureState(page, 'automatic-connecting', '#notice', [1280, 390], ['light', 'dark']);
    barrier.release();
    await page.waitForURL(`${origin}/remote/`);
  } finally {
    barrier.release();
  }
});

test('native automatic transport failure offers reopening before manual pairing', async () => {
  const page = await browser.newPage();
  const offer = await automaticOffer('/remote/settings');
  await page.route('**/sdk/pair-offer', (route) => route.abort('failed'));
  await page.goto(offer.link, { waitUntil: 'domcontentloaded' });
  await expect(page.locator('#status')).toHaveText(
    'Could not connect this browser. Open the page again from the machine with tmt remote open.',
  );
  await expect(page.locator('#status code')).toHaveText(['tmt remote open']);
  await expectUnbrokenPairingCommands(page);
  expect(page.url()).toBe(`${origin}/remote/pair`);
  await expect(page.locator('#pair')).toBeHidden();
  await expectAutomaticLabels(page, true);
  await captureState(page, 'automatic-unavailable', '#notice', [1280, 390], ['light', 'dark']);
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
      expect(page.url()).toBe(`${origin}/remote/pair`);
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
  expect(new URL(link).pathname).toBe('/remote/pair');
  expect(code).toMatch(/^[A-Z2-7]{26}$/);
  expect(link).toBe(`${origin}/remote/pair#${code}`);
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
  await expect(page.locator('.header-title')).toHaveText('Pair a browser');
  await expect(page.locator('#heading')).toHaveText('Pair this browser with tmt remote');
  await expect(page.locator('.eyebrow')).toHaveText('Browser pairing');
  await expect(page.locator('.eyebrow')).toBeVisible();
  // The page removed the fragment before anything else ran.
  await page.waitForFunction(() => location.hash === '');
  await page.fill('#name', 'E2E browser');
  await page.click('button[type="submit"]');
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

  // The certificate path exposes no caller-chosen extension and keeps the key opaque.
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
    'RemoteRegistryError',
    'ReopenSessionError',
    'bindSyncContext',
    'budget',
    'certifyKey',
    'firebaseRemote',
    'landingPage',
    'management',
    'operations',
    'pairingPage',
    'remoteRegistry',
    'reopenSession',
    'shellPage',
    'syncDocumentPath',
    'transportUrl',
    'validateSyncDiscovery',
    'validateSyncDocument',
    'validateSyncFields',
    'validateSyncInterest',
    'validateSyncMutation',
    'validateSyncQuery',
    'verifySealedObject',
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
  // Separate reads have distinct owner-clock samples; identity/activity must agree.
  const withoutObservationTime = (rows: import('../src/operations.js').RemoteAgent[]) =>
    rows.map(({ observedAtMs, ...row }) => {
      expect(Number.isSafeInteger(observedAtMs)).toBe(true);
      expect(observedAtMs).toBeGreaterThan(0);
      expect(observedAtMs).toBeLessThanOrEqual(Date.now());
      return row;
    });
  expect(withoutObservationTime(await list(app))).toEqual(
    withoutObservationTime(await list(otherTab)),
  );
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
  expect(withoutObservationTime(asked.agents)).toEqual([
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
    expect(withoutObservationTime(continued.agents)).toEqual(withoutObservationTime(asked.agents));
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
    settingsUrl: `${origin}/remote/settings`,
  });
  expect(await list(app)).toHaveLength(1);
  execFileSync(BINARY, ['settings', 'sessions-per-device', '8', '--json'], { env });
  // Reload uses the shared cookie's surviving carrier, without a session reopen workaround.
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
      "import sqlite3,sys; db=sqlite3.connect(sys.argv[1]); db.execute('UPDATE machine SET route_prefix=?', (sys.argv[2],)); db.execute('DELETE FROM _migrations WHERE version>=6'); db.execute('DROP TABLE device_last_use'); db.execute('DROP TABLE settings_designation'); db.execute('DROP TABLE management_receipts'); db.execute('DROP TABLE sessions'); db.execute('CREATE TABLE sessions(client_id TEXT PRIMARY KEY REFERENCES grants(client_id),session_id TEXT NOT NULL UNIQUE,window_id TEXT NOT NULL,grant_revision INTEGER NOT NULL,next_client_sequence TEXT NOT NULL,next_server_sequence TEXT NOT NULL)'); db.execute('ALTER TABLE operations DROP COLUMN grant_revision'); db.execute('ALTER TABLE operations DROP COLUMN session_id'); db.commit(); db.close()",
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
    const sdk = (await import('/sdk/remote-v1.js' as string)) as typeof import('../src/browser.js');
    try {
      await sdk.reopenSession(undefined, { retry: 'bounded' });
      return { reason: 'opened' };
    } catch (error) {
      if (!(error instanceof sdk.ReopenSessionError) || !(error.cause instanceof sdk.RefusalError))
        throw error;
      return { reason: error.reason, detail: error.detail, code: error.cause.code };
    }
  });
  expect(refused).toEqual({
    reason: 'revoked',
    detail: 'admission-refused',
    code: 'REMOTE_DEVICE_REVOKED',
  });
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
  const pairingBootstrap = await readFile(
    new URL('../../../rust/tmt-remote/assets/pair.js', import.meta.url),
    'utf8',
  );
  const testInfo = test.info();
  const favicon = await readFile(
    new URL('../../../../../design/browser-ui/generated/favicon.svg', import.meta.url),
  );
  const iconEvidence: { scheme: string; sha256: string; path: string }[] = [];
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
          const href = await page.locator('link[rel="icon"]').getAttribute('href');
          expect(href).toBe(`data:image/svg+xml;base64,${favicon.toString('base64')}`);
          // Decode under the actual door CSP; render the tab icon, not simulated browser chrome.
          const icon = await page.evaluate(async () => {
            const image = new Image();
            image.src = document.querySelector<HTMLLinkElement>('link[rel="icon"]')!.href;
            await image.decode();
            const canvas = document.createElement('canvas');
            canvas.width = canvas.height = 32;
            const context = canvas.getContext('2d')!;
            context.fillStyle = matchMedia('(prefers-color-scheme: dark)').matches
              ? '#111'
              : '#fff';
            context.fillRect(0, 0, 32, 32);
            context.drawImage(image, 0, 0, 32, 32);
            return {
              // Inside the mark's terminal panel, above the prompt: the panel keeps one colour.
              pixel: [...context.getImageData(3, 10, 1, 1).data],
              corner: [...context.getImageData(0, 0, 1, 1).data],
              png: canvas.toDataURL('image/png').split(',')[1]!,
            };
          });
          expect(icon.pixel).toEqual([14, 17, 22, 255]);
          expect(icon.corner).toEqual(
            colorScheme === 'light' ? [255, 255, 255, 255] : [17, 17, 17, 255],
          );
          if (state === 'landing' && width === 1440) {
            const png = Buffer.from(icon.png, 'base64');
            const path = testInfo.outputPath(`favicon-${colorScheme}-32.png`);
            await writeFile(path, png);
            await testInfo.attach(`favicon-${colorScheme}-32`, { path, contentType: 'image/png' });
            iconEvidence.push({
              scheme: colorScheme,
              sha256: createHash('sha256').update(png).digest('hex'),
              path,
            });
            await writeFile(
              testInfo.outputPath('favicon-index.json'),
              JSON.stringify(iconEvidence, null, 2),
            );
          }
          await expect(page.locator('.header-title')).toHaveText(
            state === 'error'
              ? 'Page unavailable'
              : state === 'landing'
                ? 'Remote connection'
                : 'Pair a browser',
          );
          if (state === 'error') {
            await expect(page.locator('#notice')).toContainText(
              'This page is unavailable. Open it again from the machine. Your pairing is unchanged.',
            );
            await expect(page.locator('#notice')).not.toContainText('tmt remote pair');
          }
          await expect(page.locator('.state-mark')).toHaveText(
            { landing: '', pairing: '◆', confirmation: '◆', error: '✗' }[state]!,
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
              bodySize: getComputedStyle(document.body).fontSize,
              headingSize: getComputedStyle(document.querySelector('.tmt-ui-notice-heading')!)
                .fontSize,
              opacity: style.opacity,
              overflow: document.documentElement.scrollWidth > innerWidth,
              // Playwright restores the input caret with an empty style attribute.
              inline: document.querySelectorAll('[style]:not([style=""]), style').length,
              inlineScripts: Array.from(
                document.querySelectorAll('script:not([src])'),
                (script) => script.textContent,
              ),
            };
          });
          expect(look).toMatchObject({
            paper: tokenRgb('surface', 'paper', colorScheme),
            sheet:
              state === 'pairing' || state === 'confirmation'
                ? 'rgba(0, 0, 0, 0)'
                : tokenRgb('surface', 'sheet', colorScheme),
            text: tokenRgb('color', 'text', colorScheme),
            markAboveEyebrow: true,
            radius: '0px',
            border: '0px',
            bodySize: '14px',
            headingSize: width <= 650 ? '22px' : '24px',
            opacity: '1',
            overflow: false,
            inline: 0,
          });
          expect(look.inlineScripts).toEqual(
            state === 'pairing' || state === 'confirmation' ? [pairingBootstrap] : [],
          );
          expect(look.shadow).toBe('none');
          // The landing page states connection in words only, in the neutral tone.
          if (state !== 'landing')
            await expectStateColor(page, colorScheme, state === 'error' ? 'blocked' : 'waiting');
          // Colab's header, from the same tokens: mark, product, then the page title. Inside the
          // shell it is the bar beside the rail (and Remote's module menu on the landing page);
          // the refusal page keeps the fixed full-width header.
          const header = look.header;
          const beside = width <= 900 ? 0 : state === 'landing' ? 220 + 230 : 220;
          expect(header).toMatchObject({
            width: state === 'error' ? width : width - beside,
            height: parseFloat(headerTokens[width < 480 ? 'compact-height' : 'height']!),
            top: 0,
            position: state === 'error' ? 'fixed' : 'static',
            rule: '1px',
            background: tokenRgb('surface', 'sheet', colorScheme),
            markViewBox: '0 0 96 96',
            markPaths: 4,
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
        expect((await page.goto(`${origin}/remote/`))?.status()).toBe(200);
        await inspect('landing');
        expect((await page.goto(`${origin}/remote/pair/`))?.status()).toBe(404);
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
        await expect(page.locator('.words-label')).toHaveText('Four words to compare');
        await expect(page.locator('#status')).toHaveText(
          'Waiting for confirmation in your terminal. Compare these words and confirm only if they match.',
        );
        const confirmation = await page.evaluate(() => ({
          instruction: getComputedStyle(document.getElementById('status')!).color,
          body: getComputedStyle(document.body).color,
          mark: getComputedStyle(document.getElementById('mark')!).color,
          wordsSize: parseFloat(getComputedStyle(document.getElementById('words')!).fontSize),
          outerOutline: getComputedStyle(document.getElementById('comparison')!).outlineStyle,
          wordsBorder: getComputedStyle(document.getElementById('words')!).borderTopWidth,
          wordsOutline: getComputedStyle(document.getElementById('words')!).outlineStyle,
          wordsEdge: getComputedStyle(document.getElementById('words')!).borderTopColor,
          labelInside: document
            .getElementById('words')!
            .contains(document.querySelector('.words-label')),
        }));
        expect(confirmation.instruction).toBe(confirmation.body);
        expect(confirmation.mark).toBe(tokenRgb('color', 'waiting', colorScheme));
        expect(confirmation.wordsSize).toBeGreaterThanOrEqual(24);
        expect(confirmation.labelInside).toBe(false);
        expect(confirmation.outerOutline).toBe('none');
        expect(confirmation.wordsBorder).toBe('1px');
        // Keyboard focus recolours the words' 1px rule, as a field's focus does.
        expect(confirmation.wordsOutline).toBe('none');
        expect(confirmation.wordsEdge).toBe(tokenRgb('color', 'focus', colorScheme));
        await inspect('confirmation');
        await page.locator('#heading').click();
        const pointerComparison = await page.locator('#words').evaluate((words) => {
          const style = getComputedStyle(words);
          const label = document.querySelector('.words-label')!;
          return {
            border: style.borderTopColor,
            outline: style.outlineStyle,
            labelAbove: label.getBoundingClientRect().bottom <= words.getBoundingClientRect().top,
          };
        });
        expect(pointerComparison).toEqual({
          border: tokenRgb('surface', 'rule', colorScheme),
          outline: 'none',
          labelAbove: true,
        });
        await captureState(
          page,
          'pair-comparison-pointer',
          'One light words frame; label outside and no keyboard focus ring',
          [width],
          [colorScheme],
        );
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
        expect((await page.goto(`${origin}/remote/pair/abc`))?.status()).toBe(404);
        await page.goto(`${origin}/remote/pair#BAD`);
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
  const captureRefused = async (state: string): Promise<void> => {
    const directory = process.env.TMT_REMOTE_REFUSED_CAPTURE_DIR;
    if (!directory) return;
    for (const [width, theme] of [
      [1440, 'light'],
      [390, 'dark'],
    ] as const)
      await captureState(
        page,
        state,
        'Verified refusal body and enabled Check again',
        [width],
        [theme],
        undefined,
        directory,
      );
  };
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
    const expected = ['not-accepted', 'pairing-unreadable', 'checking', 'connected'].includes(state)
      ? 0
      : 1;
    await expect(commands).toHaveCount(expected);
    expect(await commands.evaluateAll((nodes) => nodes.map((node) => node.textContent))).toEqual(
      state === 'unconfirmed' ? ['tmt remote status'] : expected ? ['tmt remote pair'] : [],
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
  await page.goto(`${origin}/remote/`);
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
    await page.goto(`${origin}/remote/`, { waitUntil: 'domcontentloaded' });
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
  await page.goto(`${origin}/remote/`);
  await expect(page.locator('#state-label')).toHaveText('Different machine');
  expect(counts()).toEqual({ mounts: 1, admissions: 0, observations: 0 });
  await captureEntry('different-machine');

  await changePin(original.machineId, original.machinePublicKey.slice(0, 31));
  reset();
  await page.goto(`${origin}/remote/`);
  await expect(page.locator('#state-label')).toHaveText('Pairing unreadable');
  expect(counts()).toEqual({ mounts: 0, admissions: 0, observations: 0 });
  await captureEntry('pairing-unreadable');

  await changePin(original.machineId, original.machinePublicKey);
  reset();
  await page.route('**/sdk/mount', (route) => route.abort('failed'));
  await page.goto(`${origin}/remote/`);
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
  await page.goto(`${origin}/remote/`);
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
    await page.goto(`${origin}/remote/`, { waitUntil: 'domcontentloaded' });
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
  await expect(page.locator('#status')).toHaveText(
    "This page's connection ended. Try again. Your pairing is unchanged.",
  );
  await expect(page.getByRole('button', { name: 'Check again', exact: true })).toBeEnabled();
  await expect(page.locator('#status code')).toHaveCount(0);
  await captureEntry('not-accepted');
  await captureRefused('entry-session-ended');
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
  await page.goto(`${origin}/remote/`);
  // The retained key now receives a verified fresh-open revocation proof, not opaque 404.
  await expect(page.locator('#state-label')).toHaveText('Not accepted');
  expect(counts()).toEqual({ mounts: 1, admissions: 1, observations: 0 });
  await expect(page.locator('#status')).toHaveText(
    'This pairing was removed or has expired. To use Remote here, run tmt remote pair on the machine.',
  );
  await expect(page.locator('#status code.tmt-ui-code')).toHaveText('tmt remote pair');
  await expect(page.getByRole('button', { name: 'Check again', exact: true })).toBeEnabled();
  await captureRefused('entry-authority-ended');
  await context.close();
  const unavailable = await browser.newContext();
  await unavailable.addInitScript(() => {
    indexedDB.open = () => {
      throw new Error('private storage diagnostic');
    };
  });
  const unavailablePage = await unavailable.newPage();
  await unavailablePage.goto(`${origin}/remote/`);
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
      session: {
        activity: {
          state: 'working',
          sinceMs: 17,
          lastActivityMs: 99,
          providers: { private: true },
        },
      },
    },
    {
      id: '00000000-0000-4000-8000-000000000002',
      name: 'Codex live',
      presence: 'active',
      runningDriver: 'codex',
      session: {
        activity: { state: 'idle', sinceMs: 0, lastActivityMs: 99, providers: { private: true } },
      },
    },
    {
      id: '00000000-0000-4000-8000-000000000003',
      name: 'Future',
      presence: 'active',
      runningDriver: 'future-driver',
      session: {
        activity: {
          state: 'unknown',
          sinceMs: null,
          lastActivityMs: 99,
          providers: { private: true },
        },
      },
    },
    {
      id: '00000000-0000-4000-8000-000000000004',
      name: 'Codex remembered',
      presence: 'offline',
      driver: 'codex',
      session: { activity: { state: 'ended', sinceMs: 33, lastActivityMs: 99, providers: {} } },
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
  const before = Date.now();
  const listed = await page.evaluate(async () => {
    const sdk = (await import('/sdk/remote-v1.js' as string)) as typeof import('../src/browser.js');
    const rows = await sdk.operations(await sdk.reopenSession()).listAgents();
    return { rows, keys: rows.map((row) => Object.hasOwn(row, 'runningDriver')) };
  });
  const observedAtMs = listed.rows[0]!.observedAtMs;
  expect(Number.isSafeInteger(observedAtMs)).toBe(true);
  expect(observedAtMs).toBeGreaterThanOrEqual(before);
  expect(observedAtMs).toBeLessThanOrEqual(Date.now());
  expect(listed.keys).toEqual([true, true, false, false]);
  expect(listed.rows).toEqual(
    identities.map((row) => ({
      id: row.id,
      name: row.name,
      presence: row.presence,
      activity: { state: row.session.activity.state, sinceMs: row.session.activity.sinceMs },
      observedAtMs,
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

async function chooseSetting(page: Page, id: string, value: string): Promise<void> {
  await page.locator(`#${id}`).click();
  await page.locator(`#${id}-options [data-value="${value}"]`).click();
  await expect(page.locator(`#${id}`)).toBeFocused();
}

test('bare pairing is read only and settings explicitly enables audited sending', async () => {
  pair = spawn(BINARY, ['pair', '--json'], { env });
  const events = lines(pair);
  const offer = await events.next();
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(offer.link as string);
  await page.fill('#name', 'Settings browser');
  await page.click('button[type="submit"]');
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
  await page.goto(`${origin}/remote/settings`);
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
  const dialogs: string[] = [];
  page.on('dialog', (dialog) => {
    dialogs.push(dialog.type());
    void dialog.dismiss();
  });
  let mutations = 0;
  page.on('request', (request) => {
    if (
      request.url().endsWith('/append') &&
      request.postDataJSON().operation === 'remote.devices.talk'
    )
      mutations++;
  });
  const enable = page.getByRole('button', {
    name: 'Enable sending for Settings browser',
    exact: true,
  });
  await page.evaluate(() => {
    const original = crypto.randomUUID;
    const fixture = window as typeof window & { confirmationIds?: number };
    fixture.confirmationIds = 0;
    crypto.randomUUID = () => {
      fixture.confirmationIds!++;
      return original.call(crypto);
    };
  });
  await enable.dblclick();
  expect(
    await page.evaluate(
      () => (window as typeof window & { confirmationIds?: number }).confirmationIds,
    ),
  ).toBe(0);
  await expect(page.locator('[data-confirmation]')).toContainText(
    'Settings browser will be able to send to its permitted agents. Press Enable sending again to confirm. You can disable sending later.',
  );
  await captureState(page, 'settings-enable-confirmation', 'Inline consequence before any effect');
  expect(mutations).toBe(0);
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(enable).toBeFocused();
  await expect(page.locator('[data-confirmation]')).toBeHidden();
  expect(mutations).toBe(0);
  await enable.focus();
  await page.keyboard.down('Enter');
  await page.keyboard.down('Enter');
  await page.keyboard.up('Enter');
  await expect(page.locator('[data-confirmation]')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(enable).toBeFocused();
  await expect(page.locator('[data-confirmation]')).toBeHidden();
  expect(mutations).toBe(0);
  expect(
    await page.evaluate(
      () => (window as typeof window & { confirmationIds?: number }).confirmationIds,
    ),
  ).toBe(0);
  await enable.click();
  await enable.click();
  expect(dialogs).toEqual([]);
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
  expect(mutations).toBe(1);
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
  await page.click('button[type="submit"]');
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
    const guidance = 'Read-only in this browser. To remove devices here, run this on the machine:';
    const help =
      "No limit by default. A limit can stop another tab from connecting; it does not remove the browser's pairing.";
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
    await page.goto(`${origin}/remote/settings`, { waitUntil: 'commit' });
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
    /^Settings browser · This device · browser · Paired · Sending on · 2 live sessions · Last used .+$/,
  );
  await expect(page.locator('#limit-value')).toHaveText('Off (unlimited) · default');
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
    await expect(page.locator('#opening')).toHaveAccessibleName('Open a browser when pairing On');
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
  await expect(page.locator('select')).toHaveCount(0);
  await page.locator('#opening').focus();
  await page.keyboard.press('ArrowDown');
  await expect(page.getByRole('option', { name: 'On', exact: true })).toBeFocused();
  await page.keyboard.press('ArrowUp');
  await expect(page.getByRole('option', { name: 'Off', exact: true })).toBeFocused();
  await page.keyboard.press('Home');
  await expect(page.getByRole('option', { name: 'On', exact: true })).toBeFocused();
  await page.keyboard.press('o');
  await expect(page.getByRole('option', { name: 'Off', exact: true })).toBeFocused();
  await page.keyboard.press('End');
  await expect(page.getByRole('option', { name: 'Off', exact: true })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.locator('#opening')).toBeFocused();
  await expect(page.locator('#opening')).toHaveJSProperty('value', 'on');
  await page.keyboard.press('Enter');
  await page.keyboard.press('End');
  await captureState(page, 'settings-choice-open', 'Named listbox; focused option, no mutation');
  await page.keyboard.press('Space');
  await expect(page.locator('#opening')).toHaveJSProperty('value', 'off');
  await expect(page.locator('#opening')).toBeFocused();
  await page.locator('#limit-mode').click();
  await expect(page.getByRole('option', { name: 'Default (off, not configured)' })).toBeDisabled();
  await page.keyboard.press('Escape');
  await expect(page.locator('#limit-mode')).toBeFocused();
  await page.keyboard.press('Space');
  await page.keyboard.press('End');
  await expect(page.getByRole('option', { name: 'Custom', exact: true })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.locator('#firestore-budget summary')).toBeFocused();
  await expect(page.locator('#limit-mode')).toHaveAttribute('aria-expanded', 'false');
  await page.locator('#opening').click();
  await page.locator(`#name-${clientId}`).click();
  await expect(page.locator(`#name-${clientId}`)).toBeFocused();
  await expect(page.locator('#opening')).toHaveAttribute('aria-expanded', 'false');
  await chooseSetting(page, 'opening', 'off');
  await page.locator('#opening').focus();
  await page.keyboard.press('Tab');
  await expect(page.locator('#opening-form button[type=submit]')).toBeFocused();
  expect(
    await page
      .locator('#opening-form button[type=submit]')
      .evaluate((control) => control.matches(':focus-visible')),
  ).toBe(true);
  await captureState(
    page,
    'settings-action-keyboard-focus',
    'Shared Action keyboard focus; native submit ownership',
  );
  await chooseSetting(page, 'opening', 'on');
  await captureState(
    page,
    'settings-designated-default',
    'Designated browser; unset unlimited default and admitted controls',
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
    /^Settings browser · This device · browser · Paired · Sending off · 1 live session · Last used .+$/,
  );
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await captureState(
      page,
      'settings-sending-disabled',
      'Explicit sending scope disabled; other device policy and original receipt retained',
    );
  }
  const enableSending = page.getByRole('button', {
    name: 'Enable sending for Settings browser',
    exact: true,
  });
  await enableSending.click();
  await enableSending.click();
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'committed');
  await page.click('#recover');
  await expect(
    page.getByRole('button', { name: 'Disable sending for Settings browser', exact: true }),
  ).toBeEnabled();
  await expect(page.locator('.device-summary').first()).toHaveText(
    /^Settings browser · This device · browser · Paired · Sending on · 1 live session · Last used .+$/,
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
  // Refresh preserves the unset source and never turns the unlimited default into an explicit cap.
  await page.click('#refresh');
  await expect(page.locator('#refresh')).toBeEnabled();
  await descriptions(true);
  expect(
    JSON.parse(execFileSync(BINARY, ['settings', '--json'], { env, encoding: 'utf8' }))
      .sessionsPerDeviceSource,
  ).toBe('default');
  await chooseSetting(page, 'limit-mode', 'custom');
  await page.fill('#limit-custom', '8');
  await page.click('#limit-form button[type=submit]');
  await expect(page.locator('#limit-value')).toHaveText('8 · settings.json');
  await captureState(
    page,
    'settings-explicit-custom',
    'Explicit custom8 differs from default source',
  );
  await chooseSetting(page, 'opening', 'off');
  await page.click('#opening-form button[type=submit]');
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'committed');
  await expect(page.locator('#opening-value')).toHaveText('Off · settings.json');
  await chooseSetting(page, 'limit-mode', 'custom');
  await page.fill('#limit-custom', '18446744073709551615');
  await page.click('#limit-form button[type=submit]');
  await expect(page.locator('#limit-value')).toHaveText('18446744073709551615 · settings.json');
  expect(execFileSync(BINARY, ['settings', '--json'], { env, encoding: 'utf8' })).toContain(
    '18446744073709551615',
  );
  await chooseSetting(page, 'limit-mode', 'off');
  await page.click('#limit-form button[type=submit]');
  await expect(page.locator('#limit-value')).toHaveText('Off (unlimited) · settings.json');
  // Role removal fences stale admitted capabilities and keeps unsent text.
  await chooseSetting(page, 'limit-mode', 'custom');
  await page.fill('#limit-custom', '19');
  execFileSync(BINARY, ['devices', 'undesignate', '--json'], { env });
  await page.click('#limit-form button[type=submit]');
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'refused');
  await expect(page.locator('#limit-custom')).toHaveValue('19');
  const roleRefresh = routeBarrier();
  const roleRefreshJoined = routeBarrier();
  let heldRoleRefresh = false;
  let roleRefreshReleased = false;
  let roleRefreshHasJoined = false;
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
      roleRefreshHasJoined = true;
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
    await expect(page.locator('#first')).toBeDisabled();
    await expect(page.locator('#more')).toBeDisabled();
    for (const button of await page
      .locator(
        '#opening-form button[type=submit], #limit-form button[type=submit], .device form button',
      )
      .all())
      await expect(button).toBeDisabled();
    await captureState(
      page,
      'settings-role-refresh-held',
      'Management refresh held; pagination and mutation actions remain disabled',
      undefined,
      undefined,
      undefined,
      undefined,
      {
        name: 'held management refresh',
        reached: heldRoleRefresh,
        released: roleRefreshReleased,
        joined: roleRefreshHasJoined,
        settled: roleRefreshSettled,
      },
    );
  } finally {
    roleRefreshReleased = true;
    roleRefresh.release();
    await roleRefreshJoined.reached;
    await settledRoleRefresh;
    await page.unroute('**/append', holdRoleRefresh);
  }
  await expect(page.locator('#read-only')).toBeVisible();
  await expect(page.locator('#limit-custom')).toHaveValue('19');
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'refused');
  // Pagination is idle again, even when hidden on this page; mutation authority is gone.
  await expect(page.locator('#first')).toBeEnabled();
  await expect(page.locator('#more')).toBeEnabled();
  await expect(page.locator('#opening')).toBeDisabled();
  await expect(page.locator('#limit-mode')).toBeDisabled();
  for (const button of await page
    .locator(
      '#opening-form button[type=submit], #limit-form button[type=submit], .device form button',
    )
    .all())
    await expect(button).toBeDisabled();
  await descriptions(false);
  await captureState(
    page,
    'settings-role-removed-draft',
    'Role removed; unsent19 draft and historical refused outcome retained',
    undefined,
    undefined,
    undefined,
    undefined,
    {
      name: 'settled role-removed refresh',
      reached: heldRoleRefresh,
      released: roleRefreshReleased,
      joined: roleRefreshHasJoined,
      settled: roleRefreshSettled,
    },
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
    // A pending effect fences Save, while native-style choice drafts stay editable.
    await expect(page.locator('#opening')).toBeEnabled();
    await chooseSetting(page, 'opening', 'on');
    await expect(page.locator('#opening-form button[type=submit]')).toBeDisabled();
    await chooseSetting(page, 'opening', 'off');
    await name.fill('Unsent next name');
    await name.evaluate((input) => {
      (input as HTMLInputElement).setSelectionRange(2, 2);
    });
  } finally {
    release();
  }
  await expect(page.locator('#outcome')).toHaveText(
    'This change could not be confirmed. Check the saved value before making another change.',
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
  await expect(page.locator('#original')).toHaveText(/^Original change [0-9a-f-]{36}$/);
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
  const revoke = page.getByRole('button', { name: 'Remove Renamed browser', exact: true });
  await page.evaluate(() => {
    const original = crypto.randomUUID;
    const fixture = window as typeof window & { confirmationIds?: number };
    fixture.confirmationIds = 0;
    crypto.randomUUID = () => {
      fixture.confirmationIds!++;
      return original.call(crypto);
    };
  });
  await revoke.dblclick();
  expect(
    await page.evaluate(
      () => (window as typeof window & { confirmationIds?: number }).confirmationIds,
    ),
  ).toBe(0);
  await expect(page.locator('[data-confirmation]')).toContainText(
    'This ends Remote access for this browser (Renamed browser). Press Remove again to confirm.',
  );
  await expect(name).toHaveValue('Unsent next name');
  expect(await name.evaluate((input) => (input as HTMLInputElement).selectionStart)).toBe(2);
  await captureState(page, 'settings-revoke-confirmation', 'Admitted name, unsent draft retained');
  expect(revokeCalls).toBe(0);
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(revoke).toBeFocused();
  expect(revokeCalls).toBe(0);
  await revoke.click();
  await revoke.click();
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
  // Verified loss is terminal: Refresh performs no read or new admission.
  let requestsAfterLoss = 0;
  const recordRequest = () => requestsAfterLoss++;
  page.on('request', recordRequest);
  await page.click('#refresh');
  await expect(page.locator('#refresh')).toBeEnabled();
  await expect(page.locator('#refresh')).toHaveAttribute('aria-busy', 'false');
  await expect(page.locator('#access')).toContainText('refused');
  expect(requestsAfterLoss).toBe(0);
  page.off('request', recordRequest);
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
  await page.click('button[type="submit"]');
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
  await page.goto(`${origin}/remote/settings`);
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
  await chooseSetting(page, 'opening', 'off');
  await page.click('#opening-form button[type=submit]');
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
  await chooseSetting(page, 'opening', 'on');
  await page.click('#opening-form button[type=submit]');
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'unknown');
  await expect(page.locator('#original')).toHaveText(/^Original change [0-9a-f-]{36}$/);
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
  const revoke = target.getByRole('button', { name: 'Remove Reviewed device', exact: true });
  await revoke.click();
  await expect(target.locator('[data-confirmation]')).toContainText(
    'This ends Remote access for Reviewed device. Press Remove again to confirm.',
  );
  await captureState(
    page,
    'settings-revoke-ready',
    'Inline revoke consequence beside admitted target',
  );
  await revoke.click();
  await expect(target).toHaveCount(0);
  const removed = JSON.parse(
    execFileSync(BINARY, ['devices', '--json'], { env, encoding: 'utf8' }),
  ) as { devices: { clientId: string }[] };
  expect(
    removed.devices.some((row) => row.clientId === '00000000-0000-4000-8000-000000000001'),
  ).toBe(false);
  await captureState(
    page,
    'settings-other-device-removed',
    'Removed device is absent from the public inventory',
  );
  // Saturate the actual retained-identity quota in this disposable Store only.
  execFileSync('python3', [
    '-c',
    "import sqlite3,sys,uuid,json;d=sqlite3.connect(sys.argv[1]);client=sys.argv[2];n=d.execute('select count(*) from management_receipts where client_id=?',(client,)).fetchone()[0];d.executemany('insert into management_receipts values(?,?,?,?,?,?,?,?)',[(str(uuid.uuid4()),client,'remote.settings.set',bytes(32),1,0,0,json.dumps({'state':'unknown','reason':'effect_outcome_unconfirmed'})) for _ in range(1000-n)]);d.commit();d.close()",
    join(root, 'state/remote/remote.db'),
    inventory.devices[0]!.clientId,
  ]);
  await chooseSetting(page, 'opening', 'off');
  await page.click('#opening-form button[type=submit]');
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'refused');
  await expect(page.locator('#controls-reason')).toHaveText(
    'Browser change limit reached. Use the local CLI; do not retry or reset storage.',
  );
  await expect(page.locator('#opening-form button[type=submit]')).toBeDisabled();
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
  await expect(page.locator('#limit-value')).toHaveText('Off (unlimited) · default');
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
  await page.click('button[type="submit"]');
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
  await page.goto(`${origin}/remote/settings`);
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
  await chooseSetting(page, 'opening', 'off');
  await page.click('#opening-form button[type=submit]');
  await expect(page.locator('#outcome')).toHaveAttribute('data-state', 'unknown');
  await captureState(
    page,
    'settings-crash-unknown',
    'Actual joined serve SIGKILL with original unknown; no resubmission',
  );
  await expect(page.locator('#original')).toHaveText(`Original change ${originalId}`);
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
  await page.goto(`${origin}/remote/settings`);
  await expect(page.locator('#access')).toHaveText(
    "Remote may have stopped, or its reply couldn't be verified. Your pairing is unchanged.",
  );
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
  await page.click('button[type="submit"]');
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
    await page.goto(`${origin}/remote/settings`);
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
  // This injects the projection; real Complete-record guard policy is covered below.
  const enabled = [
    {
      layer: 'sharing',
      state: 'enabled',
      prerequisites: ['project', 'sign-in', 'rules', 'indexes', 'plan-tier', 'quota'].map(
        (item) => ({
          item,
          state: 'enabled',
        }),
      ),
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
  await page.goto(`${origin}/remote/settings`);
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
  const projectDirectory = join(root, 'state/remote/deployments/demo-remote-fixture');
  await mkdir(projectDirectory, { recursive: true, mode: 0o700 });
  const recordPath = join(projectDirectory, 'deploy.json');
  await writeFile(recordPath, JSON.stringify(partial), { mode: 0o600 });
  await page.goto(`${origin}/remote/settings`);
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

  const legacyComplete = {
    ...partial,
    record: {
      ...partial.record,
      binding: {
        planDigest: partial.record.run.planDigest,
        project: 'demo-remote-fixture',
        completedAtMs: 2,
      },
      run: {
        ...partial.record.run,
        state: 'complete',
        steps: partial.record.run.steps.map((step) => ({ ...step, state: 'done' })),
      },
    },
  };
  const legacyBytes = JSON.stringify(legacyComplete);
  await writeFile(recordPath, legacyBytes, { mode: 0o600 });
  await page.goto(`${origin}/remote/settings`);
  const indexItem = page.locator('[data-item="indexes"]');
  await expect(indexItem).toContainText('Firestore index readiness has not been checked.');
  await expect(indexItem).toContainText('Run tmt remote deploy firestore.');
  const legacyLayers = (await recorded()).firestoreLayers as {
    prerequisites: { item: string; state: string }[];
  }[];
  expect(legacyLayers[0]!.prerequisites.find((item) => item.item === 'indexes')!.state).toBe(
    'unknown',
  );
  expect(await readFile(recordPath, 'utf8')).toBe(legacyBytes);
  await captureFirestore(
    page,
    'legacy-complete',
    1440,
    'light',
    'legacy Complete without composite evidence; unknown index; record bytes unchanged',
  );

  const complete = structuredClone(legacyComplete);
  complete.record.run.steps.splice(2, 0, {
    id: `composite-index:log:${'b'.repeat(64)}`,
    state: 'done',
  });
  const completeBytes = JSON.stringify(complete);
  await writeFile(recordPath, completeBytes, { mode: 0o600 });
  await page.goto(`${origin}/remote/settings`);
  const sharing = page.locator('[data-layer="sharing"]');
  await expect(sharing.locator('h3')).toContainText('Enabled');
  await expect(sharing).toContainText(
    "Each client stops itself before the free-plan limits. TMT does not check the project's remaining quota, so Firebase may still refuse requests.",
  );
  for (const item of ['plan-tier', 'quota'])
    await expect(page.locator(`[data-item="${item}"]`)).toContainText('Enabled');
  expect(await readFile(recordPath, 'utf8')).toBe(completeBytes);
  expect(((await recorded()).firestoreLayers as { state: string }[])[0]!.state).toBe('enabled');
  for (const width of [1440, 390]) {
    for (const theme of ['light', 'dark'] as const)
      await captureFirestore(
        page,
        'guarded-ready-record',
        width,
        theme,
        'real binary + signed SDK; private Complete/index fixture; guarded requirement, no observed headroom',
      );
  }

  for (const [state, mark, sentence, next] of [
    ['done', 'Enabled', undefined, undefined],
    [
      'building',
      'Building',
      'Firestore indexes are still building.',
      'Wait a few minutes, then run tmt remote deploy firestore.',
    ],
    [
      'pending',
      'Not checked',
      'Firestore indexes have not been verified as ready.',
      'Run tmt remote deploy firestore.',
    ],
    [
      'unknown',
      'Not checked',
      'Firestore index readiness has not been checked.',
      'Run tmt remote deploy firestore.',
    ],
  ] as const) {
    const record = structuredClone(legacyComplete);
    record.record.run.state = 'partial';
    record.record.run.rulesAttempted = false;
    record.record.run.steps.at(-1)!.state = 'pending';
    record.record.run.steps.splice(2, 0, {
      id: `composite-index:log:${'b'.repeat(64)}`,
      state,
    });
    const bytes = JSON.stringify(record);
    await writeFile(recordPath, bytes, { mode: 0o600 });
    await page.goto(`${origin}/remote/settings`);
    await expect(indexItem).toContainText('Firestore indexes:');
    await expect(indexItem.locator('.firestore-state')).toContainText(mark);
    await expect(indexItem.locator('.firestore-state [aria-hidden="true"]')).toHaveText(
      state === 'done' ? '✓' : state === 'building' ? '◌' : '○',
    );
    await expect(indexItem.locator('.firestore-state')).toHaveAttribute(
      'data-tone',
      state === 'done' ? 'working' : 'waiting',
    );
    if (sentence) {
      await expect(indexItem).toContainText(sentence);
      await expect(indexItem).toContainText(next!);
    } else expect(await indexItem.locator('p').count()).toBe(0);
    expect(await readFile(recordPath, 'utf8')).toBe(bytes);
    await captureFirestore(
      page,
      `indexes-${state}`,
      390,
      'light',
      'real binary and signed SDK; private recorded index fixture, not provider evidence',
    );
  }

  await writeFile(recordPath, 'damaged fixture', { mode: 0o600 });
  await page.goto(`${origin}/remote/settings`);
  await expect(page.locator('[data-layer="sharing"]')).toContainText('Not checked');
  expect((await recorded()).firestoreLayers).toEqual(
    expect.arrayContaining([expect.objectContaining({ layer: 'sharing', state: 'unknown' })]),
  );
  expect(await page.locator('[data-layer="sharing"] li').count()).toBe(6);
  for (const item of ['project', 'sign-in', 'rules', 'indexes'])
    await expect(page.locator(`[data-item="${item}"]`)).toContainText('Not checked');
  for (const item of ['plan-tier', 'quota'])
    await expect(page.locator(`[data-item="${item}"]`)).toContainText('Enabled');
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
  expect(await readFile(recordPath, 'utf8')).toBe('damaged fixture');
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
  await page.click('button[type="submit"]');
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
  await page.goto(`${origin}/remote/settings`);
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
    const layout = await page.evaluate(() => {
      const form = document.querySelector('#opening-form')!;
      const section = form.closest('section')!;
      const summary = section.querySelector('.settings-summary')!.getBoundingClientRect();
      const field = form.getBoundingClientRect();
      const button = form.querySelector('button[type=submit]')!.getBoundingClientRect();
      const hint = document.querySelector('#opening-unchanged')!.getBoundingClientRect();
      const choice = document.querySelector('#opening')!;
      return {
        bodySize: getComputedStyle(document.body).fontSize,
        sectionBorder: getComputedStyle(section).borderTopWidth,
        choiceHeight: choice.getBoundingClientRect().height,
        arrowPadding: getComputedStyle(choice).paddingRight,
        columns: summary.right <= field.left,
        stacked: summary.bottom <= field.top,
        feedbackGap: hint.left - button.right,
        feedbackTop: hint.top - button.top,
        noOverflow: document.documentElement.scrollWidth <= innerWidth,
      };
    });
    expect(layout).toMatchObject({
      bodySize: '14px',
      sectionBorder: '0px',
      choiceHeight: width <= 650 ? 44 : 36,
      arrowPadding: '36px',
      columns: width > 650,
      stacked: width <= 650,
      noOverflow: true,
    });
    // When there is room, the hint shares the action baseline and the approved gap.
    if (width > 650) {
      expect(layout.feedbackGap).toBe(12);
      expect(Math.abs(layout.feedbackTop)).toBeLessThanOrEqual(1);
    }

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
    await chooseSetting(page, 'opening', 'off');
    await expect(openingSave).toBeEnabled();
    await expect(page.locator('#opening-unchanged')).toBeHidden();
    await chooseSetting(page, 'opening', 'on');
    await expect(openingSave).toBeDisabled();
    await page
      .locator('#limit-form')
      .evaluate((form) =>
        scrollTo(0, form.closest('section')!.getBoundingClientRect().top + scrollY - 70),
      );
    await page.locator('#limit-form').evaluate((form) => (form as HTMLFormElement).requestSubmit());
    await feedbackBounds(page, '#limit-unchanged');
    await chooseSetting(page, 'limit-mode', 'custom');
    await page.fill('#limit-custom', '');
    await expect(limitSave).toBeDisabled();
    await page.fill('#limit-custom', '8');
    await expect(limitSave).toBeEnabled(); // Explicit 8 differs from admitted default provenance.
    await page.locator('#limit-mode').evaluate((choice) => {
      (choice as HTMLButtonElement).value = 'default';
      choice.dispatchEvent(new Event('change', { bubbles: true }));
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
    await chooseSetting(page, 'opening', 'off');
    await expect(slot).toHaveText('Device renamed.');
    await chooseSetting(page, 'opening', 'on');
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
    await page.evaluate(async () => {
      const { remoteRegistry } = await import('/sdk/remote-v1.js' as string);
      await remoteRegistry.add('foreign', 'http://127.0.0.1:9876/');
    });
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
    const renameButton = row.locator('button[type=submit]');
    const buttonLook = () =>
      renameButton.evaluate((button) => {
        const style = getComputedStyle(button);
        const box = button.getBoundingClientRect();
        return {
          color: style.color,
          background: style.backgroundColor,
          width: box.width,
          height: box.height,
        };
      });
    await page.mouse.move(0, 0);
    const idleLook = await buttonLook();
    await name.press('Enter');
    await published.reached;
    try {
      // The pressed action's delayed mark is the only busy signal: no words, no grey, no shift.
      await expect(slot).toHaveText('');
      // A click on the fenced recover action mid-write starts nothing: no request, no new words.
      const during: string[] = [];
      const record = (request: { url(): string }) => during.push(request.url());
      page.on('request', record);
      await page.evaluate(async () => {
        (document.querySelector('[data-recover]') as HTMLElement).click();
        await new Promise((resolve) => setTimeout(resolve, 250));
      });
      page.off('request', record);
      expect(during).toEqual([]);
      await expect(slot).toHaveText('');
      await expect(renameButton).toHaveAttribute('aria-busy', 'true');
      await expect(renameButton).toHaveAttribute('aria-disabled', 'true');
      expect(await renameButton.evaluate((button) => (button as HTMLButtonElement).disabled)).toBe(
        false,
      );
      await expect(renameButton.locator('.tmt-ui-action-mark')).toHaveAttribute(
        'data-busy',
        'true',
      );
      const busyLook = await buttonLook();
      expect(busyLook.color).toBe(idleLook.color);
      expect(busyLook.background).toBe(idleLook.background);
      expect(Math.abs(busyLook.width - idleLook.width)).toBeLessThanOrEqual(1);
      expect(Math.abs(busyLook.height - idleLook.height)).toBeLessThanOrEqual(1);
      if (process.env.TMT_REMOTE_CAPTURE_DIR) {
        await mkdir(process.env.TMT_REMOTE_CAPTURE_DIR, { recursive: true });
        await row.screenshot({
          path: join(
            process.env.TMT_REMOTE_CAPTURE_DIR,
            `feedback-busy-rename-${width}-${theme}.png`,
          ),
        });
      }
      await page.mouse.move(1000, 600);
      await page.locator('.tmt-ui-module-menu').hover();
      const foreign = page.locator('.tmt-ui-module-menu a', { hasText: /^Open foreign/ });
      await expect(foreign).toHaveAttribute('aria-disabled', 'true');
      await expect(foreign).toHaveAttribute(
        'title',
        'A request is in progress. Wait for its outcome.',
      );
      await name.focus();
      await expect(name).toBeFocused();
      await name.fill('Unsent next draft');
      await name.evaluate((input) => (input as HTMLInputElement).setSelectionRange(2, 2));
    } finally {
      published.release();
    }
    await joined.reached;
    await expect(slot).toHaveText(
      'This change could not be confirmed. Check the saved value before making another change.',
    );
    await expect(slot).toHaveAttribute('data-state', 'unknown');
    await expect(name).toBeFocused();
    await expect(name).toHaveValue('Unsent next draft');
    await page.locator('.tmt-ui-module-menu').hover();
    const foreign = page.locator('.tmt-ui-module-menu a', { hasText: /^Open foreign/ });
    await expect(foreign).toHaveAttribute('aria-disabled', 'true');
    await expect(foreign).toHaveAttribute(
      'title',
      'Another change is still unconfirmed. Check the saved value first.',
    );
    const menu = page.locator('.tmt-ui-module-menu');
    await expect(menu.getByRole('link', { name: 'Add remote…' })).not.toHaveAttribute(
      'aria-disabled',
    );
    await expect(menu.getByRole('link', { name: 'Remote settings' })).toBeVisible();
    // On a phone the module menu is tabs at the top; hovering it scrolled there, so return.
    await row.evaluate((form) => scrollTo(0, form.getBoundingClientRect().top + scrollY - 70));
    await name.focus();
    const originalId = writes.at(-1)!.input.operationId;
    await expect(page.locator('#original')).toHaveText(`Original change ${originalId}`);
    await expect(row.locator(`#device-reason-${targetId}`)).toBeEmpty();
    await expect(row.locator(`#device-reason-${targetId}`)).toBeHidden();
    for (const otherId of [clientId, '00000000-0000-4000-8000-000000000002'])
      await expect(page.locator(`#device-reason-${otherId}`)).toHaveText(
        'Another change is still unconfirmed. Check the saved value first.',
      );
    await expect(
      row.getByRole('button', { name: 'Check the saved value', exact: true }),
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
    await expect(page.getByRole('button', { name: 'Remove Device 1', exact: true })).toBeEnabled();
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
    await chooseSetting(page, 'limit-mode', 'custom');
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
    await expect(page.locator('#comparison')).toHaveAccessibleName('Four words to compare');
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

interface RegistryWorld {
  page: Page;
  context: import('@playwright/test').BrowserContext;
  network: string[];
  secondOrigin: string;
  secondEnv: NodeJS.ProcessEnv;
  pairAt(ownerEnv: NodeJS.ProcessEnv, label: string): Promise<void>;
  identity(): Promise<{
    paired: unknown;
    publicKey: number[];
    extractable: boolean;
    signature: number[];
  }>;
  sdk<T>(run: string, input?: unknown): Promise<T>;
}
/** Fixture mechanics shared by independent registry and static-switcher scenarios. */
async function withRegistryWorld(run: (world: RegistryWorld) => Promise<void>): Promise<void> {
  const secondRoot = await mkdtemp('/tmp/tmt-registry-');
  await writeFile(
    join(secondRoot, 'core.mjs'),
    await readFile(new URL('./core-fixture.mjs', import.meta.url)),
    { mode: 0o700 },
  );
  const secondEnv = {
    ...env,
    HOME: secondRoot,
    TMT_FIXTURE_ROOT: secondRoot,
    TMT_EXECUTABLE: join(secondRoot, 'core.mjs'),
    TMT_AGENT: '1',
    BROWSER: '/usr/bin/true',
  };
  const secondServe = spawn(BINARY, ['serve', '--json'], { env: secondEnv });
  let secondPair: ChildProcessWithoutNullStreams | undefined;
  const context = await browser.newContext();
  const page = await context.newPage();
  const network: string[] = [];
  page.on('request', (request) => network.push(request.url()));
  try {
    const secondDescriptor = await lines(secondServe).next();
    const secondOrigin = new URL(secondDescriptor.address as string).origin;
    async function pairAt(ownerEnv: NodeJS.ProcessEnv, label: string) {
      const child = spawn(BINARY, ['pair', '--json'], {
        env: { ...ownerEnv, TMT_AGENT: '1', BROWSER: '/usr/bin/true' },
      });
      secondPair = child;
      const events = lines(child),
        offer = await events.next();
      await page.goto(offer.link as string);
      await page.locator('#name').fill(label);
      await page.getByRole('button', { name: 'Pair', exact: true }).click();
      const candidate = await events.next();
      await expect(page.locator('#words')).toHaveText((candidate.words as string[]).join(' '));
      child.stdin.write('confirm\n');
      expect((await events.next()).reason).toBe('paired');
      await exited(child);
      expect(child.exitCode).toBe(0);
      await expect(page.locator('#entry-link')).toBeVisible();
    }
    async function identity() {
      return page.evaluate(async () => {
        const database = await new Promise<IDBDatabase>((resolve) => {
          const r = indexedDB.open('tmt-remote', 1);
          r.onsuccess = () => resolve(r.result);
        });
        const record = await new Promise<{
          handle: CryptoKey;
          publicKey: Uint8Array;
          paired: unknown;
        }>((resolve) => {
          const r = database.transaction('device').objectStore('device').get('device');
          r.onsuccess = () => resolve(r.result);
        });
        database.close();
        return {
          paired: record.paired,
          publicKey: Array.from(record.publicKey),
          extractable: record.handle.extractable,
          signature: Array.from(
            new Uint8Array(
              await crypto.subtle.sign(
                'Ed25519',
                record.handle,
                new TextEncoder().encode('registry identity control'),
              ),
            ),
          ),
        };
      });
    }
    async function sdk<T>(run: string, input?: unknown): Promise<T> {
      return page.evaluate(
        async ({ run, input }) => {
          const { remoteRegistry } = (await import('/sdk/remote-v1.js' as string)) as {
            remoteRegistry: Record<string, (...args: unknown[]) => Promise<unknown>>;
          };
          return remoteRegistry[run]!(...((input as unknown[] | undefined) ?? []));
        },
        { run, input },
      ) as Promise<T>;
    }
    await run({ page, context, network, secondOrigin, secondEnv, pairAt, identity, sdk });
  } finally {
    try {
      await context.close();
    } finally {
      // A browser timeout must not skip the independently owned second door's cleanup.
      secondPair?.kill('SIGTERM');
      if (secondPair) await exited(secondPair);
      secondServe.kill('SIGTERM');
      await exited(secondServe);
      await rm(secondRoot, { recursive: true, force: true });
    }
  }
}

// B1 is Remote's registry consumer, not Colab routing or a foreign health probe.
test('landing registry keeps two origins independent and confirms public-only handoffs', async () => {
  await withRegistryWorld(
    async ({ page, context, network, secondOrigin, secondEnv, pairAt, identity, sdk }) => {
      const captureDirectory = process.env.TMT_REMOTE_REGISTRY_CAPTURE_DIR;
      async function capture(state: string, lookAt: string) {
        await captureState(
          page,
          `registry-${state}`,
          lookAt,
          [1440],
          ['light'],
          undefined,
          captureDirectory,
        );
        await captureState(
          page,
          `registry-${state}`,
          lookAt,
          [390],
          ['dark'],
          undefined,
          captureDirectory,
        );
      }
      await pairAt(env, 'Registry A');
      await page.goto(`${origin}/remote/`);
      await expect(page.locator('#state-label')).toHaveText('Connected');
      const first = await identity();
      expect(first.extractable).toBe(false);
      await sdk('add', ['studio', `${origin}/`]);
      await sdk('add', ['desk', `${secondOrigin}/`]);
      await page.reload();
      await expect(page.locator('#state-label')).toHaveText('Connected');
      await expect(page.locator('[data-remote="studio"] .remote-state')).toHaveText('Connected');
      await expect(
        page.locator('[data-remote="studio"]').getByRole('button', { name: 'Open' }),
      ).toHaveCount(0);
      await expect(
        page.locator('[data-remote="studio"]').getByRole('button', { name: 'Rename studio' }),
      ).toHaveText('Rename');
      await expect(page.locator('[data-remote="desk"] .remote-state')).toHaveText('not checked');
      await expect(page.locator('[data-remote="desk"] .state-mark')).toHaveCount(0);
      expect(network.filter((url) => url.startsWith(secondOrigin))).toEqual([]);
      await page.locator('#remotes').scrollIntoViewIfNeeded();
      await capture(
        'list',
        'Current access is verified; the other origin is not probed or marked.',
      );
      await page.locator('#remote-add').click();
      await expect(page.getByRole('dialog')).toBeVisible();
      await expect(page.getByLabel('Remote name')).toBeFocused();
      await capture('add', 'Explicit public door address or remote link; no implicit sign-in.');
      await page.getByRole('button', { name: 'Cancel', exact: true }).click();
      await expect(page.locator('#remote-add')).toBeFocused();
      await page.locator('#remote-add').click();
      await page.getByLabel('Remote name').fill('spare');
      await page.getByLabel('Remote address or link').fill('http://127.0.0.1:9876/');
      await page.getByRole('button', { name: 'Continue', exact: true }).click();
      await expect(page.getByRole('heading', { name: 'Add remote spare?' })).toBeVisible();
      await page.getByRole('button', { name: 'Add', exact: true }).click();
      await expect(page.getByRole('dialog')).toHaveCount(0);
      await expect(page.locator('#remote-add')).toBeFocused();
      await expect(page.locator('[data-remote="spare"] .remote-state')).toHaveText('not checked');
      expect(network.some((url) => url.startsWith('http://127.0.0.1:9876/'))).toBe(false);
      expect(await identity()).toEqual(first);
      await sdk('remove', ['spare']);
      const openB = await sdk<string>('navigation', [
        { name: 'desk', kind: 'local', url: `${secondOrigin}/`, project: null },
      ]);
      expect(openB).not.toContain('clientId');
      expect(openB).not.toContain('machinePublicKey');
      await pairAt(secondEnv, 'Registry B');
      await page.goto(`${secondOrigin}/remote/`);
      await expect(page.locator('#state-label')).toHaveText('Connected');
      const second = await identity();
      expect(second.publicKey).not.toEqual(first.publicKey);
      await sdk('add', ['studio', `${secondOrigin}/`]);
      await page.goto(openB);
      await expect(page.getByRole('dialog')).toHaveAttribute(
        'aria-labelledby',
        'remote-dialog-heading',
      );
      await expect(
        page.getByRole('heading', { name: 'Add these remotes to this browser?' }),
      ).toBeVisible();
      expect(new URL(page.url()).hash).toBe('');
      await expect(
        page.getByText('You already have a remote named studio', { exact: true }),
      ).toBeVisible();
      await expect(page.getByLabel('Remote name')).toHaveValue('studio-2');
      await expect(page.getByText('Already added as studio', { exact: true })).toBeVisible();
      await capture('import-collision', 'Collision is renamed and the same target is skipped.');
      await page.getByRole('button', { name: 'Cancel', exact: true }).click();
      expect(await sdk('list')).toEqual({
        version: 1,
        entries: [{ name: 'studio', kind: 'local', url: `${secondOrigin}/`, project: null }],
      });
      await page.goto(openB);
      await expect(page.getByRole('dialog')).toBeVisible();
      await page.getByRole('button', { name: 'Add', exact: true }).click();
      await expect(page.getByRole('dialog')).toHaveCount(0);
      expect(await identity()).toEqual(second);
      await page.goto(openB);
      await expect(page.getByRole('button', { name: 'Add', exact: true })).toBeDisabled();
      await expect(page.getByText('Nothing new to add.', { exact: true })).toBeVisible();
      await page.getByRole('button', { name: 'Cancel', exact: true }).click();
      await page.getByRole('button', { name: 'Rename studio-2', exact: true }).click();
      await page.getByLabel('Remote name').fill('studio');
      await page.getByRole('button', { name: 'Save', exact: true }).click();
      await expect(page.getByRole('dialog').getByRole('status')).toHaveText(
        'You already have a remote named studio',
      );
      await page.getByLabel('Remote name').fill('first');
      await page.getByRole('button', { name: 'Save', exact: true }).click();
      await expect(page.getByRole('button', { name: 'Remove first', exact: true })).toBeVisible();
      await page.getByRole('button', { name: 'Remove first', exact: true }).click();
      await expect(
        page.getByRole('heading', { name: 'Remove first from this browser?' }),
      ).toBeVisible();
      await capture(
        'remove',
        'Removing public reference data does not revoke or sign out the device.',
      );
      await page.getByRole('button', { name: 'Cancel', exact: true }).click();
      await expect(page.getByRole('button', { name: 'Remove first', exact: true })).toBeFocused();
      await page.getByRole('button', { name: 'Remove first', exact: true }).click();
      await page.getByRole('button', { name: 'Remove', exact: true }).click();
      await expect(page.locator('#remote-add')).toBeFocused();
      expect(await identity()).toEqual(second);
      // A's old fixed-key device record remains readable and unchanged after a B round trip.
      await page.goto(`${origin}/remote/`);
      await expect(page.locator('#state-label')).toHaveText('Connected');
      expect(await identity()).toEqual(first);
      expect((await sdk<{ entries: unknown[] }>('list')).entries).toHaveLength(2);
      // A preview cannot overwrite a concurrent edit, and an invalid import stores no row.
      const beforeInvalid = await sdk('list');
      await page.goto(`${origin}/remote/#tmt-remotes=bad`);
      await expect(page.locator('#remote-status')).toHaveText('The remote name or link is invalid');
      expect(new URL(page.url()).hash).toBe('');
      expect(await sdk('list')).toEqual(beforeInvalid);
      const concurrent = await page.evaluate(async () => {
        const { remoteRegistry } = (await import(
          '/sdk/remote-v1.js' as string
        )) as typeof import('../src/browser.js');
        const plan = await remoteRegistry.planImport({
          version: 1,
          entries: [
            { name: 'incoming', kind: 'local', url: 'http://127.0.0.1:9123/', project: null },
          ],
        });
        await remoteRegistry.add('other', 'http://127.0.0.1:9124/');
        try {
          await remoteRegistry.confirmImport(plan, ['incoming']);
          return 'unexpected';
        } catch (error) {
          return (error as { code: string }).code;
        }
      });
      expect(concurrent).toBe('REMOTE_SELECTOR_CONFLICT');
      expect(
        (await sdk<{ entries: { name: string }[] }>('list')).entries.map((entry) => entry.name),
      ).toEqual(['studio', 'desk', 'other']);
      await sdk('remove', ['other']);
      expect(await identity()).toEqual(first);
      const corruption = await page.evaluate(async () => {
        const { remoteRegistry } = (await import(
          '/sdk/remote-v1.js' as string
        )) as typeof import('../src/browser.js');
        const db = await new Promise<IDBDatabase>((resolve) => {
          const r = indexedDB.open('tmt-remote', 1);
          r.onsuccess = () => resolve(r.result);
        });
        const original = await new Promise<unknown>((resolve) => {
          const r = db.transaction('device').objectStore('device').get('remotes-v1');
          r.onsuccess = () => resolve(r.result);
        });
        const write = (value: unknown) =>
          new Promise<void>((resolve) => {
            const tx = db.transaction('device', 'readwrite');
            tx.objectStore('device').put(value, 'remotes-v1');
            tx.oncomplete = () => resolve();
          });
        const result: string[] = [];
        for (const invalid of [
          '{"version":1,"entries":[],"device":"PRIVATE_CANARY"}',
          ' '.repeat(8193),
          '{"version":1,"entries":[],"version":1}',
        ]) {
          await write(invalid);
          try {
            await remoteRegistry.list();
            result.push('unexpected');
          } catch (error) {
            result.push((error as { code: string }).code);
          }
        }
        await write(original);
        db.close();
        return result;
      });
      expect(corruption).toEqual(Array(3).fill('REMOTE_SELECTOR_UNAVAILABLE'));
      expect(await identity()).toEqual(first);
      // Replacing only a pairing (the legacy save path) cannot clear the public catalog.
      await page.evaluate(async () => {
        const db = await new Promise<IDBDatabase>((resolve) => {
          const r = indexedDB.open('tmt-remote', 1);
          r.onsuccess = () => resolve(r.result);
        });
        const tx = db.transaction('device', 'readwrite');
        tx.objectStore('device').delete('device');
        await new Promise<void>((resolve) => {
          tx.oncomplete = () => resolve();
        });
        db.close();
      });
      expect((await sdk<{ entries: unknown[] }>('list')).entries).toHaveLength(2);
      await page.reload();
      await expect(page.locator('#state-label')).toHaveText('Not paired');
      // Browser-owned site-data clearing ends both records, unlike device-only replacement.
      const storage = await context.newCDPSession(page);
      await storage.send('Storage.clearDataForOrigin', { origin, storageTypes: 'indexeddb' });
      await storage.detach();
      expect(await sdk('list')).toEqual({ version: 1, entries: [] });
      await page.goto(`${secondOrigin}/remote/`);
      await expect(page.locator('#state-label')).toHaveText('Connected');
      expect(await identity()).toEqual(second);
      expect((await sdk<{ entries: unknown[] }>('list')).entries).toHaveLength(1);
    },
  );
});

test('the Remote module menu lists both origins and keeps settings drafts across them', async () => {
  await withRegistryWorld(async ({ page, secondOrigin, secondEnv, pairAt, identity, sdk }) => {
    await pairAt(secondEnv, 'Registry B');
    await page.goto(`${secondOrigin}/remote/`);
    await expect(page.locator('#state-label')).toHaveText('Connected');
    const second = await identity();
    await pairAt(env, 'Registry A');
    await page.goto(`${origin}/remote/`);
    await expect(page.locator('#state-label')).toHaveText('Connected');
    const first = await identity();
    const menu = page.locator('.tmt-ui-module-menu');
    await expect(menu.getByRole('link', { name: 'this machine · connected' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    await sdk('add', ['studio', `${origin}/`]);
    await sdk('add', ['desk', `${secondOrigin}/`]);
    await page.reload();
    await expect(page.locator('#state-label')).toHaveText('Connected');
    const currentItem = menu.getByRole('link', { name: 'studio · connected' });
    const foreignItem = menu.locator('a', { hasText: /^Open desk/ });
    await expect(currentItem).toHaveAttribute('aria-current', 'page');
    await expect(foreignItem).toBeVisible();
    await expect(foreignItem).not.toHaveAttribute('aria-current');
    const switchCapture = process.env.TMT_REMOTE_SWITCHER_CAPTURE_DIR;
    async function switchShot(state: string, lookAt: string) {
      await captureState(page, state, lookAt, [1440], ['light'], undefined, switchCapture);
      await captureState(page, state, lookAt, [390], ['dark'], undefined, switchCapture);
    }
    await switchShot(
      'remote-menu',
      'Current origin verified; foreign Open neutral; module menu rows and actions.',
    );
    const ownerDevices = JSON.parse(
      execFileSync(BINARY, ['devices', '--json'], { env, encoding: 'utf8' }),
    );
    execFileSync(BINARY, ['devices', 'designate', ownerDevices.devices[0].clientId, '--json'], {
      env,
    });
    await menu.getByRole('link', { name: 'Remote settings', exact: true }).click();
    await expect(page.locator('#access')).toHaveText('Current browser access confirmed.');
    await expect(page.locator('#remotes')).toBeVisible();
    await expect(page.locator('[data-remote="studio"] .remote-state')).toHaveText('Connected');
    await expect(menu.getByRole('link', { name: 'Remote settings', exact: true })).toHaveAttribute(
      'aria-current',
      'page',
    );
    await switchShot(
      'settings-remotes',
      'Same B1 registry controller reused in settings; no browser default or signed projection.',
    );
    const ownName = page.getByRole('textbox', { name: /^Device name for Registry A$/ });
    await ownName.fill('Unsent Remote name');
    await menu.hover();
    await expect(foreignItem).toHaveAttribute('aria-disabled', 'true');
    await expect(foreignItem).toHaveAttribute(
      'title',
      'Save or restore the unsent device name before changing pages.',
    );
    // Playwright refuses aria-disabled targets; press it anyway to prove it stays put.
    await foreignItem.click({ force: true });
    await expect(page).toHaveURL(`${origin}/remote/settings`);
    await ownName.fill('Registry A');
    await page.mouse.move(1000, 600);
    await menu.hover();
    await expect(foreignItem).not.toHaveAttribute('aria-disabled');
    await foreignItem.click();
    await expect(
      page.getByRole('heading', { name: 'Add these remotes to this browser?' }),
    ).toBeVisible();
    await page.getByRole('button', { name: 'Add', exact: true }).click();
    await expect(page.locator('#state-label')).toHaveText('Connected');
    expect(await identity()).toEqual(second);
    await expect(menu.getByRole('link', { name: 'desk · connected' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    await menu.locator('a', { hasText: /^Open studio/ }).click();
    await expect(page.getByRole('button', { name: 'Add', exact: true })).toBeDisabled();
    await page.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(page.locator('#state-label')).toHaveText('Connected');
    expect(await identity()).toEqual(first);
    // Attention controls are independent of the B1 browser-owned site-data clearing proof.
    await page.evaluate(async () => {
      const db = await new Promise<IDBDatabase>((resolve) => {
        const opening = indexedDB.open('tmt-remote', 1);
        opening.onsuccess = () => resolve(opening.result);
      });
      const transaction = db.transaction('device', 'readwrite');
      transaction.objectStore('device').delete('device');
      await new Promise<void>((resolve) => {
        transaction.oncomplete = () => resolve();
      });
      db.close();
    });
    await page.reload();
    await expect(page.locator('#state-label')).toHaveText('Not paired');
    await expect(menu.getByRole('link', { name: 'studio · not paired' })).toBeVisible();
    await menu.getByRole('link', { name: 'Pair again', exact: true }).click();
    await expect(page.locator('#copy-missing-pair')).toBeFocused();
  });
});

test('legacy pair redirect retains the auto fragment and erases it before every subresource', async () => {
  const page = await browser.newPage();
  const offer = await automaticOffer('/remote/');
  const link = new URL(offer.link);
  const code = link.hash.slice(6);
  expect(link.pathname).toBe('/remote/pair');
  link.pathname = '/pair';
  link.search = '?next=https://example.com/';
  const seen: { pathname: string; hash: string; automatic: string | null }[] = [];
  const requests: string[] = [];
  page.on('request', (request) => requests.push(request.url()));
  await page.route('**/sdk/**', async (route) => {
    const state = await page.evaluate(() => ({
      pathname: location.pathname,
      hash: location.hash,
      automatic: document.documentElement.getAttribute('data-tmt-auto'),
    }));
    if (state.pathname === '/remote/pair') seen.push(state);
    await route.continue();
  });
  try {
    const response = await page.goto(link.href, { waitUntil: 'domcontentloaded' });
    expect(response?.request().redirectedFrom()?.url()).toBe(`${origin}/pair${link.search}`);
    await page.waitForURL(`${origin}/remote/`);
    expect(seen.length).toBeGreaterThan(0);
    expect(seen.every((state) => state.hash === '' && state.automatic === 'true')).toBe(true);
    expect(requests.every((request) => !request.includes(code))).toBe(true);
    const devices = (
      JSON.parse(execFileSync(BINARY, ['devices', '--json'], { env, encoding: 'utf8' })) as {
        devices: { scopes: string[] }[];
      }
    ).devices;
    expect(devices).toHaveLength(1);
    expect(devices[0]!.scopes).toEqual([
      'agents.read',
      'check.read',
      'results.read',
      'status.read',
    ]);
    // Join the initial-resource observation before history navigation replaces its document.
    await page.unrouteAll({ behavior: 'wait' });
    await page.goBack();
    expect(page.url()).not.toContain('#');
    await page.goForward();
    expect(page.url()).not.toContain('#');
  } finally {
    await page.close();
  }
});

test('Home and TMT settings render in the shell and fit both sizes and themes', async () => {
  const page = await browser.newPage();
  const calls: string[] = [];
  page.on('request', (request) => calls.push(request.url()));
  try {
    await page.goto(`${origin}/`);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Home');
    const rail = page.getByRole('navigation', { name: 'TMT', exact: true });
    await expect(rail.getByRole('link', { name: 'Home', exact: true })).toHaveAttribute(
      'aria-current',
      'page',
    );
    await expect(rail.getByRole('link', { name: 'Remote', exact: true })).toHaveAttribute(
      'href',
      `${origin}/remote/`,
    );
    await expect(rail.getByRole('link', { name: 'TMT settings', exact: true })).toHaveAttribute(
      'href',
      `${origin}/settings/`,
    );
    await expect(page.getByRole('heading', { name: '◆ Needs you' })).toBeVisible();
    await expect(page.locator('#home-needs-note')).not.toHaveText('Checking…');
    await expect(page.locator('#home-recent-block')).toBeHidden();
    expect(await page.locator('script').count()).toBe(1);
    await captureState(
      page,
      'home-shell',
      'Rail with Home current; Needs-you rows only from described counts; no recent pages',
      [1440, 390],
      ['light', 'dark'],
    );
    await rail.getByRole('link', { name: 'TMT settings', exact: true }).click();
    // The settings page is sectioned: the module menu lists the sections and the title names
    // the current one.
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('General');
    const sections = page.locator('.tmt-ui-module-menu');
    await expect(sections).toContainText('TMT settings');
    for (const label of ['General', 'Identities', 'Modules', 'Drivers & hooks'])
      await expect(sections.getByRole('link', { name: label, exact: true })).toBeVisible();
    await expect(sections.getByRole('link', { name: 'General', exact: true })).toHaveAttribute(
      'aria-current',
      'page',
    );
    await expect(rail.getByRole('link', { name: 'TMT settings', exact: true })).toHaveAttribute(
      'aria-current',
      'page',
    );
    await captureState(
      page,
      'tmt-settings-shell',
      'Sectioned TMT settings: theme choice and marked sample data',
      [1440, 390],
      ['light', 'dark'],
    // The theme is a per-browser choice that applies at once and can return to the device.
    const themes = page.getByRole('tablist', { name: 'Theme' });
    await expect(themes.getByRole('tab', { selected: true })).toHaveText('System');
    await themes.getByRole('tab', { name: 'Dark', exact: true }).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    await themes.getByRole('tab', { name: 'System', exact: true }).click();
    await expect(page.locator('html')).not.toHaveAttribute('data-theme');
    // Without a Core read the sections show marked sample data, never invented as real.
    await expect(page.locator('#tmt-settings-note')).toContainText('Sample data');
    );
    // The only route-prefixed read is the owner's composed shell (display data).
    expect(
      calls
        .map((url) => new URL(url).pathname)
        .filter((path) => path.startsWith('/r/'))
        .every((path) => path.endsWith('/x/-/shell')),
    ).toBe(true);
    await rail.getByRole('link', { name: 'Home', exact: true }).click();
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Home');
  } finally {
    await page.close();
  }
});

test('the rail shows the same items in the same order on every page', async () => {
  const page = await browser.newPage();
  try {
    const rails: string[][] = [];
    for (const path of ['/', '/remote/', '/remote/settings', '/ops/', '/digest/', '/settings/']) {
      await page.goto(`${origin}${path}`);
      const items = page.locator('.tmt-ui-rail .tmt-ui-rail-item');
      await expect(items).toHaveCount(7);
      rails.push(await items.evaluateAll((nodes) => nodes.map((node) => node.dataset.id!)));
    }
    for (const rail of rails)
      expect(rail).toEqual(['home', 'colab', 'remote', 'ops', 'digest', 'search', 'settings']);
  } finally {
    await page.close();
  }
});

test('Digest and Ops render their read-only web views from sample sources in the shell', async () => {
  const page = await browser.newPage();
  try {
    await page.goto(`${origin}/digest/`);
    const rail = page.getByRole('navigation', { name: 'TMT', exact: true });
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Digest');
    await expect(rail.getByRole('link', { name: 'Digest', exact: true })).toHaveAttribute(
      'aria-current',
      'page',
    );
    // Sample data says so; the spec's six ranges, 7d first; one row per agent.
    await expect(page.locator('#usage-note')).toContainText('Sample data');
    const ranges = page.getByRole('tab');
    await expect(ranges).toHaveText(['5m', '1h', '6h', '1d', '7d', '30d']);
    await expect(page.getByRole('tab', { name: '7d' })).toHaveAttribute('aria-selected', 'true');
    await expect(page.locator('#usage-rows tr')).toHaveCount(5);
    const share = async () =>
      (await page.locator('#usage-rows tr td:nth-child(3)').allTextContents()).join(' ');
    const week = await share();
    await page.getByRole('tab', { name: '30d' }).click();
    await expect(page.getByRole('tab', { name: '30d' })).toHaveAttribute('aria-selected', 'true');
    expect(await share()).toBe(week);
    await captureState(
      page,
      'digest-web',
      'Sample usage table with ranges',
      [1440, 390],
      ['light', 'dark'],
    );
    await rail.getByRole('link', { name: 'Ops', exact: true }).click();
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Agents');
    await expect(page.locator('#ops-note')).toContainText('Sample data');
    const menu = page.locator('.tmt-ui-module-menu');
    await expect(menu.getByRole('link', { name: 'Agents', exact: true })).toHaveAttribute(
      'aria-current',
      'page',
    );
    // Every agent row is a mark and a word.
    await expect(page.locator('#ops-section .tmt-ui-list-row').first()).toContainText('Working');
    await menu.getByRole('link', { name: 'Squads', exact: true }).click();
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Squads');
    await expect(page.locator('#ops-section')).toContainText('lead tmt-ux-lead');
    await menu.getByRole('link', { name: 'Timers', exact: true }).click();
    await expect(page.locator('#ops-section')).toContainText('hourly :07');
    await captureState(
      page,
      'ops-web',
      'Sample board: schedules section',
      [1440, 390],
      ['light', 'dark'],
    );
  } finally {
    await page.close();
  }
});

test('permanent pairing short-circuits manual offers and atomically removes the other devices', async () => {
  const context = await browser.newContext();
  const page = await context.newPage();
  const first = await automaticOffer('/remote/');
  await page.goto(first.link);
  await page.waitForURL(`${origin}/remote/`);
  const inventory = () =>
    JSON.parse(execFileSync(BINARY, ['devices', '--json'], { env, encoding: 'utf8' })) as {
      devices: { clientId: string }[];
    };
  const keeper = inventory().devices[0]!.clientId;
  const seed = (start: number, count: number) =>
    execFileSync('python3', [
      '-c',
      "import sqlite3,sys;db=sqlite3.connect(sys.argv[1]);start=int(sys.argv[3]);count=int(sys.argv[4]);db.executemany('INSERT INTO grants VALUES (?,?,?,?,?,?,?,?,?,?,?,?)',[('00000000-0000-4000-8000-%012d'%i,i.to_bytes(32,'big'),'browser',sys.argv[2],'Old browser %d'%i,'all','agents.read','direct',0,None,1,0) for i in range(start,start+count)]);db.commit();db.close()",
      join(root, 'state/remote/remote.db'),
      origin,
      String(start),
      String(count),
    ]);
  seed(1, 60);
  await page.goto(`${origin}/remote/settings`);
  await expect(page.locator('#designation-code')).toHaveText(
    `tmt remote devices designate ${keeper}`,
  );
  await expect(page.locator('#read-only')).toHaveText(
    'Read-only in this browser. To remove devices here, run this on the machine:',
  );
  await expect(page.locator('#remove-others')).toBeDisabled();
  await expect(page.locator('#limit-value')).toHaveText('Off (unlimited) · default');
  await captureState(
    page,
    'permanent-pairing-read-only',
    'Default unlimited; exact local designation command',
  );
  execFileSync(BINARY, ['devices', 'designate', keeper, '--json'], { env });
  // Same origin-local key must not answer a new manual offer or notify its terminal.
  pair = spawn(BINARY, ['pair', '--json'], { env });
  let terminalOutput = '';
  pair.stdout.on('data', (chunk) => {
    terminalOutput += String(chunk);
  });
  const events = lines(pair);
  const offer = await events.next();
  // Observe the real redirect, then abort just that navigation to capture its
  // preceding status. Holding a navigation prevents Playwright from reading the DOM.
  const redirected = page.waitForEvent('requestfailed', {
    predicate: (request) => request.isNavigationRequest() && request.url() === `${origin}/remote/`,
  });
  await page.route(`${origin}/remote/`, (route) => route.abort('aborted'));
  await page.goto(offer.link as string, { waitUntil: 'domcontentloaded' });
  await redirected;
  await expect(page.locator('#status')).toHaveText('This browser is already paired.');
  await captureState(
    page,
    'permanent-pairing-already-paired',
    'Real short-circuit status; attempted continuation observed and aborted only for capture',
  );
  const stillOpen = await page.request.get(`${origin}/sdk/pair-offer`);
  expect(stillOpen.status()).toBe(200);
  expect((await stillOpen.json()).offerId).toBe((offer.descriptor as { offerId: string }).offerId);
  await page.unroute(`${origin}/remote/`);
  await page.goto(`${origin}/remote/`);
  // With no candidate, pair has not begun reading a terminal answer. Closing
  // only this private client cancels its offer; it must have printed no new event.
  pair.kill('SIGTERM');
  await exited(pair);
  expect(
    terminalOutput
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line).event),
  ).toEqual(['offer']);
  pair = undefined;
  expect(inventory().devices.filter((row) => row.clientId === keeper)).toHaveLength(1);
  await page.goto(`${origin}/remote/settings`);
  await expect(page.locator('#remove-others')).toBeEnabled();
  let commits = 0;
  page.on('request', (request) => {
    try {
      if (JSON.parse(request.postData() ?? '{}').operation === 'remote.devices.remove-others')
        commits++;
    } catch {}
  });
  await page.locator('#remove-others').dblclick();
  await expect(page.locator('#remove-others-consequence')).toHaveText(
    'This ends Remote access for 60 other devices. This browser stays connected. Press Remove the others again to confirm.',
  );
  expect(commits).toBe(0);
  expect(inventory().devices).toHaveLength(61);
  await captureState(
    page,
    'permanent-pairing-remove-others-plural',
    'Read-only preview across three device pages; no mutation on double-click',
  );
  execFileSync(BINARY, ['devices', 'revoke', '00000000-0000-4000-8000-000000000001', '--json'], {
    env,
  });
  await page.locator('#remove-others').click();
  await expect(page.locator('#outcome')).toHaveText(
    'The device list changed. Review it, then confirm again.',
  );
  expect(commits).toBe(1);
  expect(inventory().devices).toHaveLength(60);
  await page.locator('#remove-others').click();
  await expect(page.locator('#remove-others-consequence')).toContainText('59 other devices');
  await page.locator('#remove-others').click();
  await expect(page.locator('.device')).toHaveCount(1);
  expect(inventory().devices.map((row) => row.clientId)).toEqual([keeper]);
  expect(commits).toBe(2);
  await captureState(
    page,
    'permanent-pairing-others-removed',
    'Atomic removal across pages; caller grant and designation retained',
  );
  seed(1001, 1);
  await page.locator('#refresh').click();
  await expect(page.locator('#remove-others')).toBeEnabled();
  await page.locator('#remove-others').click();
  await expect(page.locator('#remove-others-consequence')).toHaveText(
    'This ends Remote access for 1 other device. This browser stays connected. Press Remove the others again to confirm.',
  );
  await captureState(page, 'permanent-pairing-remove-others-singular', 'Singular confirmation');
  await page.locator('#remove-others').click();
  await expect(page.locator('.device')).toHaveCount(1);
  expect(inventory().devices.map((row) => row.clientId)).toEqual([keeper]);
  expect(commits).toBe(3);
  await context.close();
});
