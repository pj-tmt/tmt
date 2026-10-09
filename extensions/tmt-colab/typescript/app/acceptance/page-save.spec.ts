import { expect, test, type Page } from '@playwright/test';
import { createHash } from 'node:crypto';
import { mkdirSync } from 'node:fs';
import { text } from '../src/strings.js';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openPage, run } from './harness/ask.js';
import { distinct, openSource, save } from './harness/page-source.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

// #2032: the browser Save publishes the whole source through native preparation over the owner
// socket, so a page and a save share one limit (2 MiB) and the CLI and the browser see the same
// bytes. Every size here is incompressible, so it is the size on the wire.
const LIMIT = 2 * 1024 * 1024;
/** The alert at the 390px look, light and dark, for the UX review. */
async function capture(page: Page, name: string) {
  if (!process.env.COLAB_SAVE_CAPTURE_DIR) return;
  mkdirSync(process.env.COLAB_SAVE_CAPTURE_DIR, { recursive: true });
  await page.setViewportSize({ width: 390, height: 900 });
  for (const theme of ['light', 'dark']) {
    await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
    await page.mouse.move(389, 899);
    await page.screenshot({
      path: `${process.env.COLAB_SAVE_CAPTURE_DIR}/${name}-390-${theme}.png`,
    });
  }
  await page.setViewportSize({ width: 1280, height: 720 });
}
test.afterEach(disposeActiveWorlds);

// #1998: a single committed batch must not exhaust the live peer queue before it can ACK.
test('wide CLI and browser batches keep the original live socket and exact source through a following edit', async () => {
  test.setTimeout(300_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const browser = await pairBrowser(world, 'batch-live-author');
    const colab = world.binaries.colab;
    const original = '<h1>Before batch</h1>';
    const created = createPage(world, 'Live batch', original);
    const page = await browser.context.newPage();
    const sockets: { closed: boolean; received: number; errors: string[] }[] = [];
    page.on('websocket', (socket) => {
      if (!new URL(socket.url()).pathname.endsWith('/sync')) return;
      const observed = { closed: false, received: 0, errors: [] as string[] };
      sockets.push(observed);
      socket.on('close', () => {
        observed.closed = true;
      });
      socket.on('framereceived', ({ payload }) => {
        observed.received++;
        const frame = JSON.parse(String(payload)) as { type: string; code?: string };
        if (frame.type === 'error') observed.errors.push(frame.code ?? 'unknown');
      });
    });
    await page.goto(`${door.address}/${created.path}`);
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Before batch', exact: true }),
    ).toBeVisible();
    const box = await openSource(page);
    await expect(box).toHaveValue(original);
    expect(sockets).toHaveLength(1);
    expect(sockets[0].received).toBeGreaterThan(0);
    const sameSocket = () => {
      expect(sockets).toHaveLength(1);
      expect(sockets[0].closed).toBe(false);
      expect(sockets[0].errors).toEqual([]);
    };
    const read = () =>
      JSON.parse(run(world, colab, ['page', 'read', created.pageId, '--json'])) as {
        source: string;
      };
    const cliSource = distinct(1.5 * 1024 * 1024, '1998-cli');
    const written = JSON.parse(
      run(world, colab, ['page', 'write', created.pageId, '--file', '-', '--json'], cliSource),
    ) as { count: number };
    expect(written.count).toBeGreaterThan(4);
    await expect
      .poll(async () => (await box.inputValue()) === cliSource, { timeout: 60_000 })
      .toBe(true);
    expect(read().source === cliSource).toBe(true);
    sameSocket();
    const browserSource = distinct(1.5 * 1024 * 1024, '1998-browser');
    await box.fill(browserSource);
    await save(page);
    await expect.poll(() => read().source === browserSource, { timeout: 60_000 }).toBe(true);
    await expect
      .poll(async () => (await box.inputValue()) === browserSource, { timeout: 60_000 })
      .toBe(true);
    await expect(page.getByRole('alert')).toHaveCount(0);
    sameSocket();
    const follow = browserSource + '<h2>Following edit</h2>';
    await box.fill(follow);
    await save(page);
    await expect.poll(() => read().source === follow, { timeout: 60_000 }).toBe(true);
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Following edit', exact: true }),
    ).toBeVisible();
    sameSocket();
  });
});
test('a source over the page limit is refused with both sizes and changes nothing', async () => {
  test.setTimeout(300_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const browser = await pairBrowser(world, 'save-limit');
    const colab = world.binaries.colab;
    // When the first frame of a save and its last chunk leave the page: the upload window.
    let uploadStart = 0,
      uploadEnd = 0;
    await browser.context.routeWebSocket(/\/sync/, (ws) => {
      const server = ws.connectToServer();
      ws.onMessage((message) => {
        const type = (JSON.parse(String(message)) as { type: string }).type;
        if (type === 'save') uploadStart = Date.now();
        if (type === 'chunk') uploadEnd = Date.now();
        server.send(message);
      });
      server.onMessage((message) => ws.send(message));
    });
    const original = '<p>within the limit</p>';
    const created = createPage(world, 'Save limit', original);
    const page = await openPage(door, browser, created);
    const box = await openSource(page);
    await expect(box).toHaveValue(original);
    await box.fill('x'.repeat(LIMIT + 1));
    await save(page);
    await expect(page.getByRole('alert')).toContainText(text.saveTooLarge(LIMIT + 1, LIMIT));
    await expect(page.getByRole('alert')).toContainText(text.saveNotSaved);
    await expect(page.getByRole('alert')).toContainText('2,097,153 bytes');
    await expect(page.getByRole('alert')).toContainText('2,097,152 bytes');
    const after = JSON.parse(run(world, colab, ['page', 'read', created.pageId, '--json'])) as {
      source: string;
    };
    expect(after.source).toBe(original);
    // Exactly the limit is accepted.
    const exact = '<p>' + 'y'.repeat(LIMIT - 7) + '</p>';
    await box.fill(exact);
    await save(page);
    await expect
      .poll(
        () =>
          (
            JSON.parse(run(world, colab, ['page', 'read', created.pageId, '--json'])) as {
              source: string;
            }
          ).source.length,
        { timeout: 60_000 },
      )
      .toBe(LIMIT);
    // The 2 MiB upload fits the server's upload window with room to spare.
    const uploadMs = uploadEnd - uploadStart;
    console.log(`SAVE_UPLOAD 2 MiB upload window used ${uploadMs} ms`);
    expect(uploadMs).toBeGreaterThan(0);
    expect(uploadMs).toBeLessThan(5_000);
  });
});

test('a lost reply is settled by one status request and the save lands exactly once', async () => {
  test.setTimeout(300_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const browser = await pairBrowser(world, 'save-lost');
    const colab = world.binaries.colab;
    const sent: string[] = [];
    let dropped = 0;
    await browser.context.routeWebSocket(/\/sync/, (ws) => {
      const server = ws.connectToServer();
      ws.onMessage((message) => {
        const type = (JSON.parse(String(message)) as { type: string }).type;
        if (type === 'save' || type === 'savestatus') sent.push(type);
        server.send(message);
      });
      server.onMessage((message) => {
        if ((JSON.parse(String(message)) as { type: string }).type === 'saveresult' && !dropped) {
          dropped++;
          ws.close();
          return;
        }
        ws.send(message);
      });
    });
    const created = createPage(world, 'Save lost', '<p>before</p>');
    const page = await openPage(door, browser, created);
    const box = await openSource(page);
    await expect(box).toHaveValue('<p>before</p>');
    const next = '<p>after a lost reply</p>';
    await box.fill(next);
    await save(page);
    await expect(box).toHaveValue(next);
    await expect(page.frameLocator('iframe').getByText('after a lost reply')).toBeVisible();
    await expect(page.getByRole('alert')).toHaveCount(0);
    expect(dropped).toBe(1);
    // Exactly one status request, and the save itself is never sent a second time.
    await expect.poll(() => sent).toEqual(['save', 'savestatus']);
    await expect(page.getByRole('button', { name: text.saving, exact: true })).toHaveCount(0);
    const page2 = JSON.parse(run(world, colab, ['page', 'read', created.pageId, '--json'])) as {
      source: string;
    };
    expect(page2.source).toBe(next);
  });
});

test('a save on a stale base is refused and the newer version stays', async () => {
  test.setTimeout(300_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const browser = await pairBrowser(world, 'save-stale');
    const colab = world.binaries.colab;
    let hold = false;
    await browser.context.routeWebSocket(/\/sync/, (ws) => {
      const server = ws.connectToServer();
      ws.onMessage((message) => server.send(message));
      server.onMessage((message) => {
        // While held, the browser misses broadcasts, so its base falls behind the page.
        if (!hold || (JSON.parse(String(message)) as { type: string }).type !== 'broadcast')
          ws.send(message);
      });
    });
    const created = createPage(world, 'Save stale', '<p>v1</p>');
    const page = await openPage(door, browser, created);
    const box = await openSource(page);
    await expect(box).toHaveValue('<p>v1</p>');
    hold = true;
    run(
      world,
      colab,
      ['page', 'write', created.pageId, '--file', '-', '--json'],
      '<p>v2 by CLI</p>',
    );
    await box.fill('<p>v1 edited in the browser</p>');
    await save(page);
    await expect(page.getByRole('alert')).toContainText(text.saveStale);
    await expect(page.getByRole('alert')).toContainText(text.saveNotSaved);
    await expect(box).toHaveValue('<p>v1 edited in the browser</p>');
    await expect(page.getByRole('alert')).not.toBeFocused();
    await capture(page, 'save-stale');
    const after = JSON.parse(run(world, colab, ['page', 'read', created.pageId, '--json'])) as {
      source: string;
    };
    expect(after.source).toBe('<p>v2 by CLI</p>');
  });
});

test('an unconfirmed save names its operation, keeps the typed text and never resends', async () => {
  test.setTimeout(300_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const browser = await pairBrowser(world, 'save-unknown');
    const colab = world.binaries.colab;
    const sent: string[] = [];
    let lost = 0;
    await browser.context.routeWebSocket(/\/sync/, (ws) => {
      const server = ws.connectToServer();
      ws.onMessage((message) => {
        const type = (JSON.parse(String(message)) as { type: string }).type;
        if (type === 'save' || type === 'savestatus') sent.push(type);
        server.send(message);
      });
      server.onMessage((message) => {
        // The reply to the save and then the reply to its status request are both lost.
        if ((JSON.parse(String(message)) as { type: string }).type === 'saveresult' && lost < 2) {
          lost++;
          ws.close();
          return;
        }
        ws.send(message);
      });
    });
    const created = createPage(world, 'Save unknown', '<p>before</p>');
    const page = await openPage(door, browser, created);
    const box = await openSource(page);
    await expect(box).toHaveValue('<p>before</p>');
    const typed = '<p>typed while the connection dropped</p>';
    await box.fill(typed);
    await page.getByRole('button', { name: text.save, exact: true }).click();
    const alert = page.getByRole('alert');
    await expect(alert).toContainText(text.saveUnconfirmed);
    await expect(alert).toContainText('The connection dropped before the save was confirmed.');
    await expect(alert).toContainText(/Reference: [0-9a-f-]{36}/);
    await expect(box).toHaveValue(typed);
    await expect(alert).not.toBeFocused();
    expect(sent).toEqual(['save', 'savestatus']);
    await capture(page, 'save-unknown');
    const after = JSON.parse(run(world, colab, ['page', 'read', created.pageId, '--json'])) as {
      source: string;
    };
    // The page really took the save; only its confirmation was lost twice.
    expect(after.source).toBe(typed);
  });
});

// #1627: both whole-source writers must replace a near-cap page, then keep taking edits.
test('near-cap CLI and browser whole replacements keep taking edits and refuse oversize without changes', async () => {
  test.setTimeout(600_000);
  await withWorld(async (world) => {
    world.linkExtensions();
    console.log(`FINAL_CAP world ${world.root}`);
    const cli = (args: string[], input?: string) =>
      run(world, world.binaries.tmt, ['colab', ...args], input);
    const created = JSON.parse(
      cli(
        ['page', 'create', '--title', 'Final cap', '--file', '-', '--json'],
        '<p>small original</p>',
      ),
    ) as { pageId: string; path: string };
    const read = () =>
      JSON.parse(cli(['page', 'read', created.pageId, '--json'])) as {
        source: string;
        revision: string;
      };
    const hash = (s: string) => createHash('sha256').update(s).digest('hex');
    const verify = (step: string, expected: string, started: number) => {
      const actual = read();
      expect(actual.source === expected).toBe(true);
      expect(Buffer.byteLength(actual.source)).toBe(Buffer.byteLength(expected));
      console.log(
        `FINAL_CAP ${JSON.stringify({ step, bytes: Buffer.byteLength(expected), sha256: hash(expected), revision: actual.revision, elapsedMs: Date.now() - started })}`,
      );
    };
    const door = await startDoor(world, await freePort());
    const browser = await pairBrowser(world, 'final-cap-author');
    // A whole replacement from a small source, not an append to a large baseline.
    const a = distinct(LIMIT - 4096, '1627-cli-a');
    let started = Date.now();
    const receipt = JSON.parse(cli(['page', 'write', created.pageId, '--file', '-', '--json'], a));
    console.log(`FINAL_CAP cli receipt ${JSON.stringify(receipt)}`);
    verify('CLI whole replacement', a, started);
    const page = await openPage(door, browser, created);
    let box = await openSource(page);
    await expect.poll(async () => (await box.inputValue()) === a, { timeout: 60_000 }).toBe(true);
    const paste = async (s: string) =>
      box.evaluate((el, value) => {
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(
          el,
          value,
        );
        el.dispatchEvent(new Event('input', { bubbles: true }));
      }, s);
    const b = distinct(LIMIT - 2048, '1627-browser-b');
    await paste(b);
    started = Date.now();
    await save(page);
    await expect.poll(() => read().source === b, { timeout: 60_000 }).toBe(true);
    verify('browser whole replacement', b, started);
    await expect(page.getByRole('alert')).toHaveCount(0);
    const c = b.slice(0, -4) + 'n'.repeat(2048) + '</p>';
    expect(Buffer.byteLength(c)).toBe(LIMIT);
    await paste(c);
    started = Date.now();
    await save(page);
    await expect.poll(() => read().source === c, { timeout: 60_000 }).toBe(true);
    verify('browser follow-up at exact cap', c, started);
    await expect(page.getByRole('alert')).toHaveCount(0);
    const d = c.replace('nnnn', 'EDIT');
    expect(Buffer.byteLength(d)).toBe(LIMIT);
    started = Date.now();
    cli(['page', 'write', created.pageId, '--file', '-', '--json'], d);
    verify('CLI follow-up at exact cap', d, started);
    await page.reload();
    box = await openSource(page);
    await expect.poll(async () => (await box.inputValue()) === d, { timeout: 60_000 }).toBe(true);
    verify('reopened browser and CLI', d, Date.now());
    const acceptedRevision = read().revision;
    await paste(d + 'x');
    await save(page);
    const alert = page.getByRole('alert');
    await expect(alert).toContainText(text.saveTooLarge(LIMIT + 1, LIMIT));
    await expect(alert).toContainText(text.saveNotSaved);
    await expect(alert).toContainText('2,097,153 bytes');
    await expect(alert).toContainText('2,097,152 bytes');
    expect((await box.inputValue()) === d + 'x').toBe(true);
    expect(read().revision).toBe(acceptedRevision);
    verify('over-cap browser leaves exact source', d, Date.now());
    let refusal = '';
    try {
      cli(['page', 'write', created.pageId, '--file', '-', '--json'], d + 'x');
    } catch (error) {
      refusal = (error as Error).message;
    }
    expect(refusal).toContain('COLAB_CAPACITY');
    expect(refusal).toContain('2097153');
    expect(refusal).toContain('2097152');
    console.log(`FINAL_CAP CLI over-cap refusal ${refusal}`);
    expect(read().revision).toBe(acceptedRevision);
    verify('over-cap CLI leaves exact source', d, Date.now());
  });
});
