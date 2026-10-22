import { homeList } from '../test/home-list.js';
import { pageAction } from '../test/page-actions.js';
import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import type { Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';

let capture: Server;
let origin: string;
const requests: string[] = [];
const referrers: (string | undefined)[] = [];
test.beforeAll(async () => {
  capture = createServer((req, res) => {
    requests.push(req.url ?? '');
    referrers.push(req.headers.referer);
    res.setHeader('Access-Control-Allow-Origin', '*');
    res.setHeader('Content-Type', 'text/html');
    res.end('<h1>A captured request</h1>');
  });
  await new Promise<void>((resolve) => capture.listen(0, '127.0.0.1', resolve));
  origin = `http://127.0.0.1:${(capture.address() as AddressInfo).port}`;
});
test.afterAll(async () => {
  await new Promise<void>((resolve, reject) =>
    capture.close((err) => (err ? reject(err) : resolve())),
  );
});
test.beforeEach(() => {
  requests.length = 0;
  referrers.length = 0;
});

async function mount(page: Page, source: string) {
  return page.evaluate(
    async ({ source }) => {
      const path = '/src/renderer.ts';
      const { mountRenderer } = await import(path);
      // The app behind a probe is the space home, taller than the viewport: the probe leads. The
      // standalone page keeps the header's inset as body padding; the app behind the probe now
      // sits in the TMT shell, which drops that rule, so the probe states it itself.
      document.body.style.paddingTop =
        'calc(var(--colab-header-height) + env(safe-area-inset-top, 0px))';
      const host = document.createElement('div');
      host.id = 'probe';
      document.body.prepend(host);
      const controller = new AbortController();
      const renderer = await mountRenderer(host, source, {
        signal: controller.signal,
        onState: (state: string) => {
          host.dataset.state = state;
        },
        onAnchors: (resolved: string[]) => {
          host.dataset.resolved = JSON.stringify(resolved);
        },
        onSlots: (slots: unknown[]) => {
          host.dataset.slots = JSON.stringify(slots);
        },
        onQuestions: (questions: unknown[], reviews: unknown[]) => {
          host.dataset.questions = JSON.stringify(questions);
          host.dataset.reviews = JSON.stringify(reviews);
        },
        onReadings: (readings: unknown[]) => {
          host.dataset.readings = JSON.stringify(readings);
        },
        onOpenThread: (id: string) => {
          host.dataset.opened = id;
        },
        onToggleSelectionMode: () => {
          host.dataset.toggles = String(Number(host.dataset.toggles ?? '0') + 1);
        },
        onKey: (key: string) => {
          host.dataset.keys = JSON.stringify([
            ...(JSON.parse(host.dataset.keys ?? '[]') as string[]),
            key,
          ]);
        },
        onAnnotate: () => {
          host.dataset.annotates = String(Number(host.dataset.annotates ?? '0') + 1);
        },
        onSelection: (text: string, selector: unknown, rectangle: unknown) => {
          host.dataset.rectangle = JSON.stringify(rectangle);
          host.dataset.selector = JSON.stringify(selector);
          host.dataset.selection = text;
          host.dataset.selections = JSON.stringify([
            ...(JSON.parse(host.dataset.selections ?? '[]') as string[]),
            text,
          ]);
        },
      });
      // Give legacy scenario inputs explicit paint-only ordinals at the fixture boundary.
      const handle = {
        ...renderer,
        highlight(
          input: { id: string; selector: unknown; ordinal?: number }[],
          draft?: unknown,
          count?: number,
        ) {
          return renderer.highlight(
            input.map((anchor, index) => ({ ...anchor, ordinal: anchor.ordinal ?? index + 1 })),
            draft,
            count ?? input.length,
          );
        },
      };
      // Test-owned handles never exist in the production entry point.
      Object.assign(window, { probe: { handle, controller } });
      return { ...handle.snapshot } as {
        renderId: string;
        sourceDigest: string;
        source: string;
      };
    },
    { source },
  );
}

test('space home opens a local page, scripts run, source stays in trusted chrome', async ({
  page,
}) => {
  await page.goto('/');
  await expect(homeList(page)).toBeVisible();
  await page.screenshot({ path: '/tmp/1187-home.png', fullPage: true });
  await page.getByRole('link', { name: /A shared page/ }).click();
  await expect(page.locator('.status').filter({ hasText: 'Live' })).toBeVisible();
  const frame = page.frameLocator('iframe');
  await expect(frame.getByRole('heading', { name: 'Small pages, shared ideas.' })).toBeVisible();
  await frame.getByRole('button', { name: 'Try the page: 0' }).click();
  await expect(frame.getByRole('button', { name: 'Try the page: 1' })).toBeVisible();
  await expect(page.locator('iframe')).toHaveAttribute('sandbox', 'allow-scripts');
  await expect(page.locator('iframe')).toHaveAttribute('referrerpolicy', 'no-referrer');
  await expect(page.locator('iframe')).not.toHaveAttribute('srcdoc');
  await expect(page.locator('iframe')).toHaveAttribute('src', /\/renderer\.html$/);
  await page.screenshot({ path: '/tmp/1187-page-view.png', fullPage: true });
  await (await pageAction(page, 'Source')).click();
  await expect(page.getByRole('textbox')).toHaveAttribute('readonly', '');
  await page.screenshot({ path: '/tmp/1187-page-light.png', fullPage: true });
  const renderId = await page.locator('iframe').getAttribute('data-render-id');
  await (await pageAction(page, 'Theme: Dark')).click();
  await expect(frame.getByRole('button', { name: 'Try the page: 1' })).toBeVisible();
  await expect(page.locator('iframe')).toHaveAttribute('data-render-id', renderId!);
  // Wait for the child compositor to paint after the parent theme update.
  await frame.getByRole('button', { name: 'Try the page: 1' }).evaluate(
    (button) =>
      new Promise<void>((resolve) => {
        const view = button.ownerDocument.defaultView!;
        view.requestAnimationFrame(() => view.requestAnimationFrame(() => resolve()));
      }),
  );
  await page.screenshot({ path: '/tmp/1187-page-dark.png', fullPage: true });
  await page.getByRole('link', { name: /^Colab · local/ }).click();
  await expect(page.locator('iframe')).toHaveCount(0);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole('link', { name: /A shared page/ }).click();
  await expect(page.frameLocator('iframe').getByRole('heading')).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({ path: '/tmp/1187-page-mobile.png', fullPage: true });
});

test('opaque frame cannot read app storage, cookies, native keys or bridge; no network/top/popup/form authority', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(async () => {
    localStorage.setItem('app-secret', 'do-not-expose');
    document.cookie = 'app-session=secret; SameSite=Strict';
    const key = await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, false, [
      'encrypt',
    ]);
    Object.assign(window, {
      appKey: key,
      bridge: () => {
        document.title = 'dispatched';
      },
    });
  });
  // A real successful external request proves the capture server is not blind.
  await page.evaluate((url) => fetch(url + '/positive-control'), origin);
  expect(requests).toEqual(['/positive-control']);
  await mount(
    page,
    `<p id="proof">running</p><script>
    const denied = fn => { try { fn(); return false; } catch { return true; } };
    const result = {
      storage: denied(() => localStorage.getItem('app-secret')),
      indexedDB: denied(() => indexedDB.open('app-keys')),
      cookie: denied(() => document.cookie),
      parent: denied(() => parent.document.title),
      key: denied(() => parent.appKey),
      bridge: denied(() => parent.bridge()),
      popup: (() => {
        try { return window.open('${origin}/popup') === null; }
        catch (error) { return error instanceof DOMException && ['SecurityError', 'InvalidAccessError'].includes(error.name); }
      })(),
      top: denied(() => top.location.href = '${origin}/top'),
    };
    const image = new Image(); image.src = '${origin}/image'; document.body.append(image);
    const nested = document.createElement('iframe'); nested.src = '${origin}/nested'; document.body.append(nested);
    const form = document.createElement('form'); form.action = '${origin}/form'; document.body.append(form); form.submit();
    const sheet = document.createElement('link'); sheet.rel = 'stylesheet'; sheet.href = '${origin}/style'; document.head.append(sheet);
    const script = document.createElement('script'); script.src = '${origin}/script'; document.head.append(script);
    parent.postMessage({type:'dispatch', operation:'forged'}, '*');
    fetch('${origin}/fetch').then(() => { result.network=false; }, () => { result.network=true; }).finally(() => { document.getElementById('proof').textContent=JSON.stringify(result); });
  </script>`,
  );
  const proof = page.frameLocator('#probe iframe').locator('#proof');
  await expect(proof).toContainText('"network":true');
  expect(JSON.parse((await proof.textContent())!)).toEqual({
    storage: true,
    indexedDB: true,
    cookie: true,
    parent: true,
    key: true,
    bridge: true,
    popup: true,
    top: true,
    network: true,
  });
  expect(requests).toEqual(['/positive-control']);
  await expect(page).not.toHaveTitle('dispatched');
  await expect(homeList(page)).toBeVisible();
});

test('selection mode annotates a link before ordinary link interception', async ({ page }) => {
  await page.goto('/');
  await mount(page, '<p><a href="https://example.com/">Comment on this link</a></p>');
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  await page.evaluate(() => {
    (
      window as unknown as { probe: { handle: { selectionMode(value: boolean): void } } }
    ).probe.handle.selectionMode(true);
  });
  await page.frameLocator('#probe iframe').getByRole('link').click();
  await expect(page.locator('#probe')).toHaveAttribute('data-selection', 'Comment on this link');
  await expect(page.locator('#probe')).toHaveAttribute('data-annotates', '1');
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
});

test('self-navigation can leak one request, then tears down the frame and never resumes it', async ({
  page,
}) => {
  await page.goto('/');
  await mount(
    page,
    `<button onclick="location.href='${origin}/self-navigation'">Leave frame</button>`,
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  await page.frameLocator('#probe iframe').getByRole('button', { name: 'Leave frame' }).click();
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'navigation');
  await expect(page.locator('#probe iframe')).toHaveCount(0);
  expect(requests).toEqual(['/self-navigation']);
  expect(referrers).toEqual([undefined]);
  // no-referrer also applies to the navigation that the sandbox cannot prevent.
  await expect(homeList(page)).toBeVisible();
});

test('a page whose document takes over five seconds to load still binds, and is not failed', async ({
  page,
}) => {
  test.setTimeout(60_000);
  await page.goto('/');
  // The author's script holds the frame's `load` for 6.5 s, as a very large page does while it
  // parses. The bind acknowledgement follows it, so it arrives after the old 5 s deadline.
  const started = Date.now();
  await mount(
    page,
    '<p id="slow">Heavy page</p><script>for (const t = Date.now(); Date.now() - t < 6500; );</script>',
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready', { timeout: 30_000 });
  expect(Date.now() - started).toBeGreaterThan(5000);
  await expect(page.frameLocator('#probe iframe').locator('#slow')).toBeVisible();
});

test('handshake binds window source and renderId; captured digest and cleanup survive replacement', async ({
  page,
}) => {
  await page.goto('/');
  // Delay the real document so the wrong-window positive-shape reply is tested
  // before the renderer can acknowledge. Use real postMessage delivery on all engines.
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route('**/renderer.html', async (route) => {
    await gate;
    await route.continue();
  });
  const source =
    '<p>First</p><script>parent.postMessage({type:"colab.render.bound",renderId:"stale-id"},"*")</script>';
  const captured = await mount(page, source);
  await page.evaluate(() => {
    const admission: (string | undefined)[] = [];
    Object.assign(window, { admission });
    window.addEventListener('message', (event) => {
      if (event.data?.type === 'colab.render.bound')
        admission.push(document.getElementById('probe')!.dataset.state);
    });
  });
  try {
    await page.evaluate(async (renderId) => {
      const sibling = document.createElement('iframe');
      const received = new Promise<void>((resolve) => {
        const listener = (event: MessageEvent) => {
          if (event.source !== sibling.contentWindow) return;
          window.removeEventListener('message', listener);
          resolve();
        };
        window.addEventListener('message', listener);
      });
      sibling.srcdoc = `<script>parent.postMessage({type:'colab.render.bound',renderId:${JSON.stringify(renderId)}},'*')</script>`;
      document.body.append(sibling);
      await received;
      sibling.remove();
    }, captured.renderId);
    await expect(page.locator('#probe')).not.toHaveAttribute('data-state');
  } finally {
    release();
  }
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  expect(
    await page.evaluate(
      () => (window as unknown as { admission: (string | undefined)[] }).admission,
    ),
  ).toEqual([undefined, undefined, 'ready']);
  await expect(page.frameLocator('#probe iframe').getByText('First')).toBeVisible();
  const digest = await page.evaluate(
    async (source) =>
      [...new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(source)))]
        .map((v) => v.toString(16).padStart(2, '0'))
        .join(''),
    source,
  );
  expect(captured.sourceDigest).toBe(digest);
  expect(captured.renderId.endsWith(':' + digest)).toBe(true);
  await expect(page.locator('#probe iframe')).toHaveAttribute('data-source-digest', digest);
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  await page.evaluate(() => {
    const probe = (window as unknown as { probe: { controller: AbortController } }).probe;
    probe.controller.abort();
    document.getElementById('probe')!.remove();
  });
  expect(await page.locator('iframe').count()).toBe(0);
  const replacement = await mount(page, '<p>Replacement</p>');
  expect(replacement.renderId).not.toBe(captured.renderId);
  expect(replacement.sourceDigest).not.toBe(captured.sourceDigest);
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  await expect(page.frameLocator('#probe iframe').getByText('Replacement')).toBeVisible();
});

test('a delayed first height report keeps window scrolling performed after replacement', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(async () => {
    document.getElementById('root')!.hidden = true;
    const host = document.createElement('div');
    host.id = 'height-probe';
    document.body.append(host);
    const previous = document.createElement('iframe');
    previous.style.height = '4000px';
    host.append(previous);
    const heights: unknown[] = [];
    const hold = (event: MessageEvent) => {
      if (
        event.source === host.querySelector('iframe')?.contentWindow &&
        event.data?.type === 'colab.render.height'
      ) {
        heights.push(event.data);
        event.stopImmediatePropagation();
      }
    };
    window.addEventListener('message', hold, true);
    const path = '/src/renderer.ts';
    const { mountRenderer } = await import(path);
    const controller = new AbortController();
    const handle = await mountRenderer(host, '<div style="height:3500px">Updated page</div>', {
      signal: controller.signal,
      onState: (state: string) => {
        host.dataset.state = state;
      },
    });
    Object.assign(window, { heightProbe: { heights, hold, controller, handle } });
  });
  await expect(page.locator('#height-probe')).toHaveAttribute('data-state', 'ready');
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { heightProbe: { heights: unknown[] } }).heightProbe.heights.length,
      ),
    )
    .toBeGreaterThan(0);
  await page.evaluate(() => window.scrollTo(0, 1200));
  expect(await page.evaluate(() => window.scrollY)).toBe(1200);
  const report = await page.evaluate(() => {
    const probe = (
      window as unknown as {
        heightProbe: { heights: unknown[]; hold: (event: MessageEvent) => void };
      }
    ).heightProbe;
    window.removeEventListener('message', probe.hold, true);
    return probe.heights[0];
  });
  await page
    .frameLocator('#height-probe iframe')
    .locator('body')
    .evaluate((_node, report) => {
      parent.postMessage(report, '*');
    }, report);
  await expect
    .poll(async () => (await page.locator('#height-probe iframe').boundingBox())?.height ?? 0)
    .toBeLessThan(4000);
  expect(await page.evaluate(() => window.scrollY)).toBe(1200);
  await page.evaluate(() => {
    (
      window as unknown as { heightProbe: { controller: AbortController } }
    ).heightProbe.controller.abort();
    document.getElementById('height-probe')!.remove();
  });
});

test('a source document replacement tears down the renderer', async ({ page }) => {
  await page.goto('/');
  await mount(
    page,
    `<button onclick="document.open();document.write('<p>Replacement</p>');document.close()">Replace document</button>`,
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  await page
    .frameLocator('#probe iframe')
    .getByRole('button', { name: 'Replace document' })
    .click();
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'navigation');
  await expect(page.locator('#probe iframe')).toHaveCount(0);
});

test('renderer bootstrap ignores sibling and later messages; only the parent initializes it', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(async () => {
    const target = document.createElement('iframe');
    target.id = 'target';
    target.setAttribute('sandbox', 'allow-scripts');
    target.src = '/renderer.html';
    const loaded = new Promise<void>((resolve) => {
      target.onload = () => resolve();
    });
    document.body.append(target);
    await loaded;
    const sibling = document.createElement('iframe');
    sibling.id = 'sibling';
    sibling.srcdoc = '<!doctype html><p>Sibling</p>';
    const siblingLoaded = new Promise<void>((resolve) => {
      sibling.onload = () => resolve();
    });
    document.body.append(sibling);
    await siblingLoaded;
  });
  const sibling = page.frames().find((frame) => frame.url() === 'about:srcdoc')!;
  await sibling.evaluate(() => {
    const digest = 'a'.repeat(64);
    const target = parent.document.querySelector<HTMLIFrameElement>('#target')!;
    target.contentWindow!.postMessage(
      {
        type: 'colab.render.bind',
        renderId: `00000000-0000-4000-8000-000000000001:${digest}`,
        sourceDigest: digest,
        source: '<p>Wrong sender</p>',
        theme: 'light',
      },
      '*',
      [new MessageChannel().port2],
    );
  });
  // A same-target message from the parent is ordered after the sibling message.
  await page.evaluate(() => {
    const digest = 'a'.repeat(64);
    const target = document.querySelector<HTMLIFrameElement>('#target')!;
    target.contentWindow!.postMessage(
      {
        type: 'colab.render.bind',
        renderId: `00000000-0000-4000-8000-000000000001:${digest}`,
        sourceDigest: digest,
        source: '<p>Parent source</p>',
        theme: 'light',
      },
      '*',
      [new MessageChannel().port2],
    );
  });
  await expect(page.frameLocator('#target').getByText('Parent source')).toBeVisible();
  await page.evaluate(() => {
    const digest = 'b'.repeat(64);
    const target = document.querySelector<HTMLIFrameElement>('#target')!;
    target.contentWindow!.postMessage(
      {
        type: 'colab.render.bind',
        renderId: `00000000-0000-4000-8000-000000000002:${digest}`,
        sourceDigest: digest,
        source: '<p>Later source</p>',
        theme: 'dark',
      },
      '*',
      [new MessageChannel().port2],
    );
  });
  // A task in the child after message delivery proves later input had no effect.
  const target = page.frames().find((frame) => frame.url().endsWith('/renderer.html'))!;
  await target.evaluate(() => new Promise<void>((resolve) => setTimeout(resolve, 0)));
  await expect(page.frameLocator('#target').getByText('Parent source')).toBeVisible();
  await expect(page.frameLocator('#target').getByText('Later source')).toHaveCount(0);
});

/** One renderer frame, bound once by its parent with the given extra keys. */
async function bindFrame(page: Page, id: string, extra: Record<string, unknown>) {
  await page.evaluate(
    async ({ id, extra }) => {
      const target = document.createElement('iframe');
      target.id = id;
      target.setAttribute('sandbox', 'allow-scripts');
      target.src = '/renderer.html';
      await new Promise<void>((resolve) => {
        target.onload = () => resolve();
        document.body.append(target);
      });
      const digest = 'c'.repeat(64);
      target.contentWindow!.postMessage(
        {
          type: 'colab.render.bind',
          renderId: `00000000-0000-4000-8000-000000000003:${digest}`,
          sourceDigest: digest,
          source: '<p>Bound text</p>',
          theme: 'light',
          ...extra,
        },
        '*',
        [new MessageChannel().port2],
      );
    },
    { id, extra },
  );
}

test('a bind may carry a layout; only column and full select the page frame', async ({ page }) => {
  await page.goto('/');
  for (const layout of [undefined, 'freeform', 'canvas', 'column', 'full'] as const) {
    const id = `target-${layout ?? 'none'}`;
    await bindFrame(page, id, layout ? { layout } : {});
    const target = page.frameLocator(`#${id}`);
    await expect(target.getByText('Bound text')).toBeVisible();
    const framed = layout === 'column' || layout === 'full';
    if (framed) await expect(target.locator('html')).toHaveAttribute('data-colab-layout', layout);
    else expect(await target.locator('html').getAttribute('data-colab-layout')).toBeNull();
    // The page frame's layer is in the document only for a framed layout.
    const hasFrameRule = await target
      .locator('html')
      .evaluate((html) =>
        [...html.ownerDocument.styleSheets].some((sheet) =>
          [...sheet.cssRules].some(
            (rule) => rule instanceof CSSLayerBlockRule && rule.name === 'colab-frame',
          ),
        ),
      );
    expect(hasFrameRule).toBe(framed);
  }
  // An unknown layout is not a bind: the frame stays unwritten.
  await bindFrame(page, 'target-wide', { layout: 'wide' });
  // A task in the child after message delivery proves the message had no effect.
  await page
    .frameLocator('#target-wide')
    .locator('body')
    .evaluate(() => new Promise<void>((resolve) => setTimeout(resolve, 0)));
  await expect(page.frameLocator('#target-wide').getByText('Bound text')).toHaveCount(0);
});

test('a page opts in to the bundled runtime exactly as the shared marker vectors say', async ({
  page,
}) => {
  const { cases } = JSON.parse(
    readFileSync(
      new URL('../../../contracts/vectors/runtime-marker-v1.json', import.meta.url),
      'utf8',
    ),
  ) as { cases: { name: string; source: string; optsIn: boolean }[] };
  expect(cases.length).toBeGreaterThan(20);
  await page.goto('/');
  const answers = await page.evaluate(
    async (sources) => {
      const path = '/src/page-runtime.ts';
      const { wantsRuntime } = await import(path);
      return sources.map((source: string) => wantsRuntime(source) as boolean);
    },
    cases.map(({ source }) => source),
  );
  expect(cases.map(({ name }, index) => [name, answers[index]])).toEqual(
    cases.map(({ name, optsIn }) => [name, optsIn]),
  );
});

test('only an opting-in page fetches the runtime, and its frame gets it before the page runs', async ({
  page,
}) => {
  let fetches = 0;
  await page.route('**/assets/colab-runtime.css', (route) =>
    route.fulfill({ contentType: 'text/css', body: '@theme { --color-stub: #123456; }' }),
  );
  await page.route('**/assets/colab-runtime.js', (route) => {
    fetches += 1;
    return route.fulfill({
      contentType: 'text/javascript',
      body: 'window.tmt = Object.freeze({ stub: true });',
    });
  });
  await page.goto('/');
  await mount(
    page,
    '<p id="plain">No marker</p><script>document.getElementById("plain").dataset.tmt = String(typeof window.tmt)</script>',
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  const plain = page.frames().find((frame) => frame.url().endsWith('/renderer.html'))!;
  await expect(plain.locator('#plain')).toHaveAttribute('data-tmt', 'undefined');
  expect(fetches).toBe(0);
  await page.evaluate(() => {
    (window as unknown as { probe: { controller: AbortController } }).probe.controller.abort();
    document.querySelector('#probe')?.remove();
  });
  await mount(
    page,
    '<meta name="tmt-colab-runtime" content="react tailwind"><p id="opted">Marker</p><script>document.getElementById("opted").dataset.tmt = String(window.tmt && window.tmt.stub)</script>',
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  const opted = page.frames().find((frame) => frame.url().endsWith('/renderer.html'))!;
  await expect(opted.locator('#opted')).toHaveAttribute('data-tmt', 'true');
  expect(fetches).toBe(1);
});

test('an opting-in page whose runtime cannot load fails visibly instead of rendering half-working', async ({
  page,
}) => {
  await page.route('**/assets/colab-runtime.css', (route) =>
    route.fulfill({ contentType: 'text/css', body: '' }),
  );
  await page.route('**/assets/colab-runtime.js', (route) => route.fulfill({ status: 500 }));
  await page.goto('/');
  await mount(page, '<meta name="tmt-colab-runtime" content="react"><p>Needs React</p>');
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'failed');
});

test('selection admits bounded text from the current frame, rejects foreign and stale render IDs, and clears on teardown', async ({
  page,
}) => {
  await page.goto('/');
  const captured = await mount(page, '<p id="quote">An exact selected quote</p>');
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  const frame = page.frames().find((frame) => frame.url().endsWith('/renderer.html'))!;
  await frame.evaluate(() => {
    const range = document.createRange();
    range.selectNodeContents(document.getElementById('quote')!);
    getSelection()!.removeAllRanges();
    getSelection()!.addRange(range);
  });
  await expect(page.locator('#probe')).toHaveAttribute('data-selection', 'An exact selected quote');
  // Same-shape wrong-window traffic is ordered before an explicit barrier.
  await page.evaluate(async (renderId) => {
    const sibling = document.createElement('iframe');
    const received = new Promise<void>((resolve) => {
      const listener = (event: MessageEvent) => {
        if (event.source !== sibling.contentWindow) return;
        window.removeEventListener('message', listener);
        resolve();
      };
      window.addEventListener('message', listener);
    });
    sibling.srcdoc = `<script>parent.postMessage({type:'colab.render.selection',renderId:${JSON.stringify(renderId)},text:'foreign'},'*')</script>`;
    document.body.append(sibling);
    await received;
    sibling.remove();
  }, captured.renderId);
  await expect(page.locator('#probe')).toHaveAttribute('data-selection', 'An exact selected quote');
  await frame.evaluate(async (renderId) => {
    parent.postMessage({ type: 'colab.render.selection', renderId: 'stale', text: 'stale' }, '*');
    parent.postMessage({ type: 'colab.render.selection', renderId, text: 'é'.repeat(8193) }, '*');
    parent.postMessage(
      { type: 'colab.render.selection', renderId, text: 'unknown', unexpected: true },
      '*',
    );
    for (const rect of [
      { x: 0, y: 0, width: -1, height: 1 },
      { x: Infinity, y: 0, width: 1, height: 1 },
      { x: 0, y: 1000001, width: 1, height: 1 },
      { x: 0, y: 0, width: 1, height: 1, send: true },
    ])
      parent.postMessage(
        {
          type: 'colab.render.selection',
          renderId,
          text: 'bad rectangle',
          selector: { exact: 'bad rectangle', prefix: '', suffix: '' },
          rect,
        },
        '*',
      );
    // Valid final text proves the channel remains live after rejected inputs.
    parent.postMessage({ type: 'colab.render.selection', renderId, text: 'barrier' }, '*');
  }, captured.renderId);
  await expect(page.locator('#probe')).toHaveAttribute('data-selection', 'barrier');
  expect(JSON.parse((await page.locator('#probe').getAttribute('data-selections'))!)).toEqual([
    'An exact selected quote',
    'barrier',
  ]);
  await page.evaluate(() => {
    const probe = (window as unknown as { probe: { controller: AbortController } }).probe;
    probe.controller.abort();
  });
  await expect(page.locator('#probe iframe')).toHaveCount(0);
  await expect(page.locator('#probe')).toHaveAttribute('data-selection', '');
});

test('the frozen unsaved quote resolves without a thread highlight or marker', async ({ page }) => {
  await page.goto('/');
  await mount(page, '<p>Original quote</p>');
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  await page.evaluate(() => {
    const probe = (
      window as unknown as {
        probe: {
          handle: {
            highlight(anchors: [], draft: { exact: string; prefix: string; suffix: string }): void;
          };
        };
      }
    ).probe;
    probe.handle.highlight([], { exact: 'Original quote', prefix: '', suffix: '' });
  });
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', '[""]');
  await expect(page.frameLocator('#probe iframe').locator('[data-colab-thread]')).toHaveCount(0);
  expect(
    await page
      .frameLocator('#probe iframe')
      .locator('html')
      .evaluate(() => CSS.highlights?.has('colab-comments') ?? false),
  ).toBe(false);
});

test('quote selectors span tags and Unicode, preserve exact text, detach ambiguity and remap after edits', async ({
  page,
}) => {
  await page.goto('/');
  await mount(
    page,
    '<p id="quote">Before &amp; <em>🌍 exact</em> text. After</p><p id="other">Other</p>',
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  const frame = page.frames().find((f) => f.url().endsWith('/renderer.html'))!;
  await frame.evaluate(() => {
    const range = document.createRange();
    range.selectNodeContents(document.getElementById('quote')!);
    getSelection()!.removeAllRanges();
    getSelection()!.addRange(range);
  });
  await expect(page.locator('#probe')).toHaveAttribute(
    'data-selection',
    'Before & 🌍 exact text. After',
  );
  const selector = JSON.parse((await page.locator('#probe').getAttribute('data-selector'))!);
  expect(selector).toEqual({ exact: 'Before & 🌍 exact text. After', prefix: '', suffix: 'Other' });
  const id = '00000000-0000-4000-8000-000000000001:00000000-0000-4000-8000-000000000002';
  async function highlight(value: { exact: string; prefix: string; suffix: string }) {
    await page.evaluate(
      ({ id, value }) => {
        (
          window as unknown as { probe: { handle: { highlight(v: unknown[]): void } } }
        ).probe.handle.highlight([{ id, selector: value }]);
      },
      { id, value },
    );
  }
  await highlight(selector);
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', JSON.stringify([id]));
  expect(await frame.evaluate(() => CSS.highlights.get('colab-comments')?.size)).toBe(1);
  await frame.locator('#quote').evaluate((node) => {
    node.innerHTML = 'Before &amp; 🌍 <strong>exact text.</strong> After';
  });
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', JSON.stringify([id]));
  await frame.locator('#quote').evaluate((node) => {
    node.textContent = 'Changed';
  });
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', '[]');
  await frame.locator('#other').evaluate((node) => {
    node.textContent = 'same same';
  });
  await highlight({ exact: 'same', prefix: '', suffix: '' });
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', '[]');
  await highlight({ exact: 'same', prefix: 'Changed', suffix: ' same' });
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', JSON.stringify([id]));
  await page.evaluate(() =>
    (window as unknown as { probe: { controller: AbortController } }).probe.controller.abort(),
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', '[]');
});

test('hidden text is excluded; repeated or over-budget quotes detach and fallback overlays leave text intact', async ({
  page,
}) => {
  await page.goto('/');
  await mount(
    page,
    '<p id="quote">Visible <span hidden>hidden poison</span>🌍</p><script>const ignored="poison";</script><p id="many">' +
      'same '.repeat(65) +
      '</p>',
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  const frame = page.frames().find((f) => f.url().endsWith('/renderer.html'))!;
  await frame.locator('#quote').evaluate((node) => {
    const range = document.createRange();
    range.selectNodeContents(node);
    getSelection()!.removeAllRanges();
    getSelection()!.addRange(range);
  });
  await expect(page.locator('#probe')).toHaveAttribute('data-selection', 'Visible 🌍');
  const id = 'test';
  async function highlight(exact: string) {
    await page.evaluate(
      ({ id, exact }) =>
        (
          window as unknown as { probe: { handle: { highlight(v: unknown[]): void } } }
        ).probe.handle.highlight([{ id, selector: { exact, prefix: '', suffix: '' } }]),
      { id, exact },
    );
  }
  await highlight('same');
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', '[]');
  await frame.evaluate(() => {
    Object.defineProperty(window, 'Highlight', { value: undefined });
  });
  await highlight('Visible 🌍');
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', JSON.stringify([id]));
  await expect(frame.locator('[data-colab-highlight]:not([data-colab-markers])')).toHaveCount(1);
  await expect(frame.locator('#quote')).toHaveText('Visible hidden poison🌍');
  await frame.locator('#many').evaluate((node) => {
    node.textContent = 'x'.repeat(256 * 1024 + 1);
  });
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', '[]');
  await expect(frame.locator('[data-colab-highlight]')).toHaveCount(0);
});

test('anchor results accept only the current request and requested unique IDs; cosmetic claims grant no actions', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(() => {
    const Original = MessageChannel;
    Object.assign(window, {
      MessageChannel: class extends Original {
        constructor() {
          super();
          Object.assign(window, { anchorPort: this.port1 });
          const post = this.port1.postMessage.bind(this.port1);
          this.port1.postMessage = (value: unknown) => {
            Object.assign(window, { anchorRequest: value });
            post(value);
          };
        }
      },
    });
  });
  await mount(page, '<p>Visible</p>');
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  const result = await page.evaluate(() => {
    const w = window as unknown as {
      probe: { handle: { highlight(v: unknown[]): void } };
      anchorPort: MessagePort;
      anchorRequest: { renderId: string; requestId: string };
    };
    const highlight = () =>
      w.probe.handle.highlight([
        { id: 'expected', selector: { exact: 'missing', prefix: '', suffix: '' } },
      ]);
    const resolved = () => document.getElementById('probe')!.dataset.resolved;
    const emit = (value: unknown) =>
      w.anchorPort.dispatchEvent(new MessageEvent('message', { data: value }));
    highlight();
    const stale = { ...w.anchorRequest };
    highlight();
    const current = {
      type: 'colab.render.anchors',
      renderId: w.anchorRequest.renderId,
      requestId: w.anchorRequest.requestId,
      resolved: ['expected'],
    };
    const rejected = [];
    for (const data of [
      { ...current, requestId: stale.requestId },
      { ...current, renderId: 'stale' },
      { ...current, resolved: ['foreign'] },
      { ...current, resolved: ['expected', 'expected'] },
      { ...current, extra: true },
      { ...current, positions: [{ id: 'foreign', top: 2 }] },
      { ...current, positions: [{ id: 'expected', top: Infinity }] },
    ]) {
      emit(data);
      rejected.push(resolved());
    }
    emit(current);
    const open = {
      type: 'colab.render.open-thread',
      renderId: current.renderId,
      requestId: current.requestId,
      id: 'expected',
    };
    for (const forged of [
      { ...open, id: 'foreign' },
      { ...open, renderId: 'old' },
      { ...open, requestId: stale.requestId },
      { ...open, send: true },
    ])
      emit(forged);
    const openedBefore = document.getElementById('probe')!.dataset.opened;
    emit(open);
    return {
      rejected,
      positive: resolved(),
      openedBefore,
      opened: document.getElementById('probe')!.dataset.opened,
    };
  });
  expect(result).toEqual({
    rejected: ['[]', '[]', '[]', '[]', '[]', '[]', '[]'],
    positive: '["expected"]',
    openedBefore: undefined,
    opened: 'expected',
  });
  await expect(page.getByTestId('comment-thread')).toHaveCount(0);
  await expect(page.getByTestId('ask-preview')).toHaveCount(0);
});

test('range-end numbers keep distinct targets and quoted hints through resize and anchor scrolling', async ({
  page,
}) => {
  await page.goto('/');
  await mount(
    page,
    '<style>body{margin:20px;position:relative}.space{height:1800px}</style><div id="space" class="space"></div><p id="quote">Anchored line</p><div class="space"></div>',
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  await page.evaluate(() => {
    const handle = (
      window as unknown as { probe: { handle: { highlight(value: unknown[]): void } } }
    ).probe.handle;
    handle.highlight(
      ['first', 'second'].map((id) => ({
        id,
        selector: { exact: 'Anchored line', prefix: '', suffix: '' },
      })),
    );
  });
  const frame = page.frameLocator('#probe iframe');
  const markers = frame.locator('[data-colab-thread]');
  await expect(markers).toHaveCount(2);
  await expect(markers).toHaveText(['1', '2']);
  const marker = markers.filter({ hasText: /^1$/ });
  await expect(marker).toHaveAttribute('title', 'Thread 1: Anchored line');
  const alignment = await frame.locator('#quote').evaluate((node) => {
    const range = node.ownerDocument.createRange();
    range.selectNodeContents(node);
    // The marker is a 24 px hit box around a 16 px circle; align the circle, not the box.
    const circleBottom =
      node.ownerDocument.querySelector('[data-colab-thread]')!.getBoundingClientRect().bottom - 4;
    return Math.abs(circleBottom - range.getBoundingClientRect().top);
  });
  expect(alignment).toBeLessThanOrEqual(8);
  const initial = await marker.evaluate((node) => parseFloat((node as HTMLElement).style.top));
  await frame.locator('#space').evaluate((node: HTMLElement) => {
    node.style.height = '2200px';
  });
  await expect
    .poll(() => marker.evaluate((node) => parseFloat((node as HTMLElement).style.top)))
    .toBeGreaterThan(initial + 390);
  await page.evaluate(() =>
    (
      window as unknown as { probe: { handle: { scrollAnchor(id: string): void } } }
    ).probe.handle.scrollAnchor('first'),
  );
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBeGreaterThan(2100);
  expect(
    await frame.locator('html').evaluate((node) => node.ownerDocument.defaultView!.scrollY),
  ).toBe(0);
  await marker.click();
  await expect(page.locator('#probe')).toHaveAttribute('data-opened', 'first');
  await markers.filter({ hasText: /^2$/ }).click();
  await expect(page.locator('#probe')).toHaveAttribute('data-opened', 'second');
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(markers).toHaveCount(2);
  await frame.locator('#quote').evaluate((node) => {
    node.textContent = 'Changed source';
  });
  await expect(marker).toHaveCount(0);
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', '[]');
});

test('only IDs, quote selectors and paint ordinals reach author code even when a caller supplies private comment fields', async ({
  page,
}) => {
  await page.goto('/');
  await mount(
    page,
    `<p>Anchor quote</p><script>
    const every = Array.prototype.every;
    Array.prototype.every = function(callback, ...args) {
      if (this[0]?.selector && this[0]?.id) window.anchorTraffic = JSON.stringify(this);
      return every.call(this, callback, ...args);
    };
  </script>`,
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  const secret = 'PRIVATE COMMENT BODY MUST STAY IN PARENT';
  await page.evaluate((secret) => {
    const handle = (
      window as unknown as { probe: { handle: { highlight(value: unknown[]): void } } }
    ).probe.handle;
    handle.highlight([
      {
        id: 'known',
        selector: { exact: 'Anchor quote', prefix: '', suffix: '' },
        label: secret,
        body: secret,
        replies: [secret],
      },
    ]);
  }, secret);
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', '["known"]');
  const frame = page.frameLocator('#probe iframe');
  const observed = await frame
    .locator('html')
    .evaluate(
      (node) =>
        (node.ownerDocument.defaultView as unknown as { anchorTraffic: string }).anchorTraffic,
    );
  expect(JSON.parse(observed)).toEqual([
    { id: 'known', selector: { exact: 'Anchor quote', prefix: '', suffix: '' }, ordinal: 1 },
  ]);
  expect(observed).not.toContain(secret);
  await expect(frame.locator('[data-colab-thread]')).toHaveAttribute(
    'title',
    'Thread 1: Anchor quote',
  );
});

for (const backward of [false, true])
  test(`pointer selection publishes only after release, at its ${backward ? 'backward' : 'forward'} focus end`, async ({
    page,
  }) => {
    await page.goto('/');
    await mount(
      page,
      '<p id="quote" style="display:inline-block;font:16px monospace">Select these exact words</p>',
    );
    await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
    const frame = page.frames().find((frame) => frame.url().endsWith('/renderer.html'))!;
    const quote = frame.locator('#quote');
    await quote.scrollIntoViewIfNeeded();
    const box = (await quote.boundingBox())!;
    const start = backward ? box.x + box.width - 1 : box.x + 1;
    const end = backward ? box.x + 1 : box.x + box.width - 1;
    await page.mouse.move(start, box.y + box.height / 2);
    await page.mouse.down();
    try {
      await page.mouse.move(end, box.y + box.height / 2, { steps: 8 });
      expect(await frame.evaluate(() => getSelection()!.toString())).toBe(
        'Select these exact words',
      );
      await frame.evaluate(
        () =>
          new Promise<void>((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
          ),
      );
      await expect(page.locator('#probe')).not.toHaveAttribute('data-selection', /.+/);
      await page.keyboard.press('c');
      await expect(page.locator('#probe')).not.toHaveAttribute('data-annotates');
    } finally {
      await page.mouse.up();
    }
    await expect(page.locator('#probe')).toHaveAttribute(
      'data-selection',
      'Select these exact words',
    );
    const rectangle = JSON.parse((await page.locator('#probe').getAttribute('data-rectangle'))!);
    const focus = await frame.evaluate(() => {
      const selection = getSelection()!,
        end = document.createRange();
      end.setStart(selection.focusNode!, selection.focusOffset);
      end.collapse(true);
      const box = end.getBoundingClientRect();
      return { x: box.x, y: box.y, width: box.width, height: box.height };
    });
    expect(rectangle).toEqual(focus);
    // Starting another drag hides the previous affordance instead of retaining a stale quote.
    await page.mouse.move(start, box.y + box.height / 2);
    await page.mouse.down();
    await expect(page.locator('#probe')).toHaveAttribute('data-selection', '');
    await page.mouse.up();
  });

test('keyboard selection waits for key release; C annotates a completed selection or toggles mode with editing guards', async ({
  page,
}) => {
  await page.goto('/');
  await mount(
    page,
    '<p id="quote" contenteditable>Select words</p><input id="edit"><textarea id="area"></textarea><div id="editable" contenteditable>Editable words</div>',
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  const frame = page.frames().find((frame) => frame.url().endsWith('/renderer.html'))!;
  await frame.locator('#quote').focus();
  await frame
    .locator('#quote')
    .evaluate((node) => getSelection()!.setPosition(node.firstChild, node.textContent!.length));
  await page.keyboard.down('Shift');
  await page.keyboard.down('ArrowLeft');
  await expect.poll(() => frame.evaluate(() => getSelection()!.toString())).toBe('s');
  await frame.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
  await expect(page.locator('#probe')).not.toHaveAttribute('data-selection', /.+/);
  await page.keyboard.up('ArrowLeft');
  await page.keyboard.up('Shift');
  await expect(page.locator('#probe')).toHaveAttribute('data-selection', 's');
  // Editable author text supplies native keyboard selection without a browser caret-mode setting.
  // End editing before exercising the single-key annotation shortcut.
  await frame.locator('#quote').evaluate((node) => node.removeAttribute('contenteditable'));
  for (const key of ['Shift+c', 'Alt+c', 'Control+c', 'Meta+c']) await page.keyboard.press(key);
  await expect(page.locator('#probe')).not.toHaveAttribute('data-annotates');
  await expect(page.locator('#probe')).not.toHaveAttribute('data-toggles');
  // A trusted IME confirmation code on a non-editing target isolates that guard.
  await frame.evaluate(() => {
    document.addEventListener(
      'keydown',
      (event) => {
        Object.assign(window, {
          confirmationKey: { trusted: event.isTrusted, code: event.code, keyCode: event.keyCode },
        });
      },
      { capture: true, once: true },
    );
  });
  const client = await page.context().newCDPSession(page);
  await client.send('Input.dispatchKeyEvent', {
    type: 'rawKeyDown',
    key: 'c',
    code: 'KeyC',
    windowsVirtualKeyCode: 229,
  });
  await client.send('Input.dispatchKeyEvent', {
    type: 'keyUp',
    key: 'c',
    code: 'KeyC',
    windowsVirtualKeyCode: 229,
  });
  await expect(page.locator('#probe')).not.toHaveAttribute('data-annotates');
  await expect(page.locator('#probe')).not.toHaveAttribute('data-toggles');
  expect(
    await frame.evaluate(() => (window as unknown as { confirmationKey: unknown }).confirmationKey),
  ).toEqual({ trusted: true, code: 'KeyC', keyCode: 229 });
  await client.detach();
  await page.keyboard.press('c');
  await expect(page.locator('#probe')).toHaveAttribute('data-annotates', '1');
  await expect(page.locator('#probe')).not.toHaveAttribute('data-toggles');
  await page.keyboard.press('Alt+Enter');
  await expect(page.locator('#probe')).toHaveAttribute('data-annotates', '2');
  await frame.evaluate(() => getSelection()?.removeAllRanges());
  await expect(page.locator('#probe')).toHaveAttribute('data-selection', '');
  await page.keyboard.press('c');
  await expect(page.locator('#probe')).toHaveAttribute('data-toggles', '1');
  await page.keyboard.down('c');
  await expect(page.locator('#probe')).toHaveAttribute('data-toggles', '2');
  await page.keyboard.down('c');
  await page.keyboard.up('c');
  await expect(page.locator('#probe')).toHaveAttribute('data-toggles', '2');
  for (const id of ['edit', 'area', 'editable']) {
    await frame.locator(`#${id}`).focus();
    await page.keyboard.press('c');
    await page.keyboard.press('Alt+Enter');
    await expect(page.locator('#probe')).toHaveAttribute('data-annotates', '2');
    await expect(page.locator('#probe')).toHaveAttribute('data-toggles', '2');
  }
});

test('question groups and split slot geometry admit only the current closed response', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(() => {
    const Original = MessageChannel;
    Object.assign(window, {
      MessageChannel: class extends Original {
        constructor() {
          super();
          Object.assign(window, { questionPort: this.port1 });
          const post = this.port1.postMessage.bind(this.port1);
          this.port1.postMessage = (value: { type?: string }) => {
            if (value.type === 'colab.render.questions')
              Object.assign(window, { questionRequest: value });
            if (value.type === 'colab.render.slots') Object.assign(window, { splitRequest: value });
            post(value);
          };
        }
      },
    });
  });
  await mount(page, '<p>No admitted markup</p>');
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  const result = await page.evaluate(() => {
    const w = window as unknown as {
      probe: {
        handle: Pick<
          Awaited<ReturnType<typeof import('../src/renderer.js').mountRenderer>>,
          'questions' | 'slots'
        >;
      };
      questionPort: MessagePort;
      questionRequest: { renderId: string; requestId: string; groups: unknown[] };
      splitRequest: {
        renderId: string;
        requestId: string;
        slots: { before: number; height: number }[];
      };
    };
    const id = '80000000-0000-4000-8000-000000000001';
    const group = '80000000-0000-4000-8000-000000000002';
    const section = '80000000-0000-4000-8000-000000000003';
    const reading = '80000000-0000-4000-8000-000000000004';
    const readingBlocks = [{ id: reading, title: 'Read this', closed: false }];
    const blocks = [{ id, title: 'Which heading?' }];
    const groups = [{ id: group, title: 'Review', questions: [id], active: id }];
    const reviews = [{ id: section, title: 'Plan', closed: false }];
    w.probe.handle.questions(blocks, [], groups, reviews, readingBlocks);
    const stale = w.questionRequest.requestId;
    w.probe.handle.questions(blocks, [], groups, reviews, readingBlocks);
    const current = {
      type: 'colab.render.questions',
      renderId: w.questionRequest.renderId,
      requestId: w.questionRequest.requestId,
      questions: [
        { id, selector: { exact: 'Which heading?', prefix: '', suffix: '' }, threads: [] },
      ],
      reviews: [{ id: section, selector: { exact: 'Plan', prefix: '', suffix: '' }, threads: [] }],
      readings: [
        { id: reading, selector: { exact: 'Read this', prefix: '', suffix: '' }, threads: [] },
      ],
    };
    const emit = (value: unknown) =>
      w.questionPort.dispatchEvent(new MessageEvent('message', { data: value }));
    const read = () => document.getElementById('probe')!.dataset.questions;
    const readReviews = () => document.getElementById('probe')!.dataset.reviews;
    const { reviews: _omitted, ...previousShape } = current;
    const rejected = [];
    for (const bad of [
      { ...current, requestId: stale },
      { ...current, renderId: 'old' },
      { ...current, groups },
      // The previous four-key response and every unrequested, forged or repeated section.
      previousShape,
      { ...current, reviews: [{ ...current.reviews[0], answered: true }] },
      { ...current, readings: null },
      { ...current, readings: [{ ...current.readings[0], read: true }] },
      { ...current, readings: [{ ...current.readings[0], id }] },
      { ...current, readings: [{ ...current.readings[0], threads: ['foreign'] }] },
      { ...current, readings: [current.readings[0], current.readings[0]] },
      {
        ...current,
        readings: [
          { ...current.readings[0], selector: { exact: 'Forged', prefix: '', suffix: '' } },
        ],
      },
      { ...current, reviews: [{ ...current.reviews[0], id }] },
      { ...current, reviews: [{ ...current.reviews[0], threads: ['foreign'] }] },
      { ...current, reviews: [current.reviews[0], current.reviews[0]] },
      {
        ...current,
        reviews: [{ ...current.reviews[0], selector: { exact: 'Forged', prefix: '', suffix: '' } }],
      },
      { ...current, questions: [{ ...current.questions[0], answered: true }] },
      { ...current, questions: [{ ...current.questions[0], id: group }] },
      { ...current, questions: [{ ...current.questions[0], threads: ['foreign'] }] },
      { ...current, questions: [current.questions[0], current.questions[0]] },
      {
        ...current,
        questions: [
          { ...current.questions[0], selector: { exact: 'Forged', prefix: '', suffix: '' } },
        ],
      },
    ]) {
      emit(bad);
      rejected.push(read());
    }
    emit(current);
    const admitted = read();
    const admittedReviews = readReviews();
    const admittedReadings = document.getElementById('probe')!.dataset.readings;
    w.probe.handle.slots([{ id: group, height: 99999, before: 99999 }]);
    const slot = {
      id: group,
      top: 100,
      beforeTop: 20,
      descriptionHeight: 80,
      left: 24,
      width: 200,
    };
    const placement = {
      type: 'colab.render.slots',
      renderId: w.splitRequest.renderId,
      requestId: w.splitRequest.requestId,
      slots: [slot],
    };
    const geometry = () => document.getElementById('probe')!.dataset.slots;
    const rejectedGeometry = [];
    for (const bad of [
      { ...placement, requestId: stale },
      { ...placement, slots: [{ ...slot, beforeTop: 101 }] },
      { ...placement, slots: [{ ...slot, descriptionHeight: 2001 }] },
      { ...placement, slots: [{ ...slot, descriptionHeight: NaN }] },
      { ...placement, slots: [{ ...slot, beforeTop: -1 }] },
      { ...placement, slots: [{ ...slot, answer: true }] },
      { ...placement, slots: [slot, slot] },
      ...[NaN, Infinity, -1].map((left) => ({ ...placement, slots: [{ ...slot, left }] })),
      ...[NaN, Infinity, -1, 0, 1000001].map((width) => ({
        ...placement,
        slots: [{ ...slot, width }],
      })),
      { ...placement, slots: [{ ...slot, left: innerWidth, width: 1 }] },
      { ...placement, slots: [{ id: group, top: 100 }] },
      {
        ...placement,
        slots: [
          { id: group, top: 100, beforeTop: 20, descriptionHeight: 80, left: 24, wrong: 200 },
        ],
      },
    ]) {
      emit(bad);
      rejectedGeometry.push(geometry());
    }
    emit(placement);
    const placed = geometry();
    const frame = document.querySelector('iframe')!;
    const width = frame.style.width;
    frame.style.width = '10px';
    window.dispatchEvent(new Event('resize'));
    const retired = geometry();
    frame.style.width = width;
    window.dispatchEvent(new Event('resize'));
    emit(placement);
    return {
      rejected,
      admitted,
      admittedReviews,
      admittedReadings,
      groups: w.questionRequest.groups,
      rejectedGeometry,
      clamped: w.splitRequest.slots[0],
      geometry: placed,
      retired,
      restored: geometry(),
    };
  });
  expect(result.rejected).toEqual(Array(20).fill('[]'));
  expect(JSON.parse(result.admitted!)).toHaveLength(1);
  expect(JSON.parse(result.admittedReviews!)).toHaveLength(1);
  expect(JSON.parse(result.admittedReadings!)).toHaveLength(1);
  expect(result.groups).toEqual([
    {
      id: '80000000-0000-4000-8000-000000000002',
      title: 'Review',
      questions: ['80000000-0000-4000-8000-000000000001'],
      active: '80000000-0000-4000-8000-000000000001',
    },
  ]);
  expect(result.rejectedGeometry).toEqual(Array(18).fill('[]'));
  expect(result.retired).toBe('[]');
  expect(result.restored).toBe(result.geometry);
  expect(result.clamped).toEqual({
    id: '80000000-0000-4000-8000-000000000002',
    height: 2000,
    before: 2000,
  });
  expect(JSON.parse(result.geometry!)).toEqual([
    {
      id: '80000000-0000-4000-8000-000000000002',
      top: 100,
      beforeTop: 20,
      descriptionHeight: 80,
      left: 24,
      width: 200,
    },
  ]);
});

test('chip-form slots admit only the closed chip request and response', async ({ page }) => {
  await page.goto('/');
  await page.evaluate(() => {
    const Original = MessageChannel;
    Object.assign(window, {
      MessageChannel: class extends Original {
        constructor() {
          super();
          Object.assign(window, { chipPort: this.port1 });
          const post = this.port1.postMessage.bind(this.port1);
          this.port1.postMessage = (value: { type?: string }) => {
            if (value.type === 'colab.render.questions')
              Object.assign(window, { questionRequest: value });
            if (value.type === 'colab.render.slots') Object.assign(window, { slotRequest: value });
            post(value);
          };
        }
      },
    });
  });
  await mount(page, '<p>No admitted markup</p>');
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  const result = await page.evaluate(() => {
    const w = window as unknown as {
      probe: {
        handle: Pick<
          Awaited<ReturnType<typeof import('../src/renderer.js').mountRenderer>>,
          'questions' | 'slots'
        >;
      };
      chipPort: MessagePort;
      questionRequest: { groups: unknown[] };
      slotRequest: { renderId: string; requestId: string; slots: unknown[] };
    };
    const id = '80000000-0000-4000-8000-000000000011';
    const group = '80000000-0000-4000-8000-000000000012';
    const lone = '80000000-0000-4000-8000-000000000014';
    const blocks = [
      { id, title: 'Which heading?' },
      { id: lone, title: 'Another?' },
    ];
    const groups = [
      { id: group, title: 'Review', questions: [id], active: id, form: 'chip' as const },
      { id: lone, title: null, questions: [lone], active: lone },
    ];
    // An unknown form is refused whole; the admitted chip form reaches the frame.
    w.probe.handle.questions(blocks, [], [{ ...groups[0], form: 'inline' as 'chip' }, groups[1]]);
    const refused = w.questionRequest === undefined;
    w.probe.handle.questions(blocks, [], groups);
    // A chip size on a block-form group, or alongside `before`, is not a request.
    w.probe.handle.slots([{ id: lone, height: 10, chip: { width: 100, height: 20 } }]);
    const blockRequest = w.slotRequest === undefined;
    w.probe.handle.slots([{ id: group, height: 10, before: 5, chip: { width: 100, height: 20 } }]);
    const mixedRequest = w.slotRequest === undefined;
    w.probe.handle.slots([{ id: group, height: 0, chip: { width: 120.2, height: 24.5 } }]);
    const request = w.slotRequest;
    const placement = {
      type: 'colab.render.slots',
      renderId: request.renderId,
      requestId: request.requestId,
    };
    const slot = {
      id: group,
      top: 100,
      chip: { top: 40, left: 50, width: 121, height: 25 },
      left: 24,
      width: 200,
    };
    const geometry = () => document.getElementById('probe')!.dataset.slots;
    const emit = (value: unknown) =>
      w.chipPort.dispatchEvent(new MessageEvent('message', { data: value }));
    const rejected = [];
    for (const bad of [
      { ...placement, slots: [{ id: group, top: 100 }] },
      { ...placement, slots: [{ ...slot, chip: undefined }] },
      { ...placement, slots: [{ ...slot, chip: { ...slot.chip, extra: 1 } }] },
      { ...placement, slots: [{ ...slot, chip: { top: 1, left: 1, width: 1 } }] },
      ...[0, -1, NaN, Infinity, 1000001].map((width) => ({
        ...placement,
        slots: [{ ...slot, chip: { ...slot.chip, width } }],
      })),
      ...[-1, NaN, 1000001].map((top) => ({
        ...placement,
        slots: [{ ...slot, chip: { ...slot.chip, top } }],
      })),
      { ...placement, slots: [{ ...slot, left: -1 }] },
      { ...placement, slots: [{ ...slot, width: 0 }] },
      {
        ...placement,
        slots: [
          { id: group, top: 100, beforeTop: 20, descriptionHeight: 80, left: 24, width: 200 },
        ],
      },
      { ...placement, slots: [{ ...slot, answer: true }] },
    ]) {
      emit(bad);
      rejected.push(geometry());
    }
    emit({ ...placement, slots: [slot] });
    return {
      refused,
      blockRequest,
      mixedRequest,
      groups: w.questionRequest.groups,
      slots: request.slots,
      rejected,
      placed: geometry(),
    };
  });
  expect(result.refused).toBe(true);
  expect(result.blockRequest).toBe(true);
  expect(result.mixedRequest).toBe(true);
  expect(result.groups).toEqual([
    {
      id: '80000000-0000-4000-8000-000000000012',
      title: 'Review',
      questions: ['80000000-0000-4000-8000-000000000011'],
      active: '80000000-0000-4000-8000-000000000011',
      form: 'chip',
    },
    {
      id: '80000000-0000-4000-8000-000000000014',
      title: null,
      questions: ['80000000-0000-4000-8000-000000000014'],
      active: '80000000-0000-4000-8000-000000000014',
    },
  ]);
  // The size is rounded up and clamped before it leaves the parent.
  expect(result.slots).toEqual([
    {
      id: '80000000-0000-4000-8000-000000000012',
      height: 0,
      chip: { width: 121, height: 25 },
    },
  ]);
  expect(result.rejected).toEqual(Array(result.rejected.length).fill('[]'));
  expect(JSON.parse(result.placed!)).toEqual([
    {
      id: '80000000-0000-4000-8000-000000000012',
      top: 100,
      chip: { top: 40, left: 50, width: 121, height: 25 },
      left: 24,
      width: 200,
    },
  ]);
});

test('a chip slot asked for before its group is registered is placed once the group is', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(() => {
    const Original = MessageChannel;
    Object.assign(window, {
      MessageChannel: class extends Original {
        constructor() {
          super();
          const post = this.port1.postMessage.bind(this.port1);
          const sent: { type?: string; slots?: unknown[] }[] = [];
          Object.assign(window, { sent });
          this.port1.postMessage = (value: { type?: string }) => {
            sent.push(structuredClone(value));
            post(value);
          };
        }
      },
    });
  });
  await mount(
    page,
    '<p>Before.</p><tmt-question data-id="80000000-0000-4000-8000-000000000011" data-kind="short"><p data-question>Which heading?</p></tmt-question>',
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  const slots = await page.evaluate(() => {
    const w = window as unknown as {
      probe: {
        handle: Pick<
          Awaited<ReturnType<typeof import('../src/renderer.js').mountRenderer>>,
          'questions' | 'slots'
        >;
      };
      sent: { type?: string; slots?: unknown[] }[];
    };
    const id = '80000000-0000-4000-8000-000000000011';
    // The page measures its compact block as soon as the frame is ready, which can be before the
    // frame has been told the group: that request is not lost, it waits for the group.
    w.probe.handle.slots([{ id, height: 20, chip: { width: 120, height: 24 } }]);
    w.probe.handle.questions(
      [{ id, title: 'Which heading?' }],
      [],
      [{ id, title: null, questions: [id], active: id, form: 'chip' }],
    );
    return w.sent
      .filter((value) => value.type === 'colab.render.slots')
      .map((value) => value.slots);
  });
  expect(slots).toEqual([
    [{ id: '80000000-0000-4000-8000-000000000011', height: 20, chip: { width: 120, height: 24 } }],
  ]);
});

test('a measured page asked before the frame knows its blocks still gets every placement', async ({
  page,
}) => {
  await page.goto('/');
  const lone = '80000000-0000-4000-8000-000000000021';
  const group = '80000000-0000-4000-8000-000000000022';
  const first = '80000000-0000-4000-8000-000000000023';
  const second = '80000000-0000-4000-8000-000000000024';
  const review = '80000000-0000-4000-8000-000000000025';
  const member = (id: string, kind: string) =>
    `<tmt-question data-id="${id}" data-kind="${kind}" data-agent="a"><p data-question>Question ${id.slice(-2)}?</p></tmt-question>`;
  await mount(
    page,
    `<p>Before.</p>${member(lone, 'short')}<p>Between.</p><tmt-questions data-id="${group}"><p data-title>Group</p>${member(first, 'short')}${member(second, 'short')}</tmt-questions><p>Then.</p><tmt-review data-id="${review}" data-agent="a"><h2 data-title>Section</h2><p>Body.</p></tmt-review><p>After.</p>`,
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  await page.evaluate(
    ([lone, group, first, second, review]) => {
      const w = window as unknown as {
        probe: {
          handle: Pick<
            Awaited<ReturnType<typeof import('../src/renderer.js').mountRenderer>>,
            'questions' | 'slots'
          >;
        };
      };
      const chip = { width: 120, height: 24 };
      // The compact blocks and the section are measured the moment the frame is ready, before
      // the frame has been told which blocks and groups the page holds.
      w.probe.handle.slots([
        { id: lone, height: 20, chip },
        { id: group, height: 20, chip },
        { id: review, height: 30 },
      ]);
      w.probe.handle.questions(
        [lone, first, second].map((id) => ({ id, title: `Question ${id.slice(-2)}?` })),
        [],
        [
          { id: lone, title: null, questions: [lone], active: lone, form: 'chip' as const },
          {
            id: group,
            title: 'Group',
            questions: [first, second],
            active: first,
            form: 'chip' as const,
          },
        ],
        [{ id: review, title: 'Section', closed: false }],
      );
    },
    [lone, group, first, second, review],
  );
  await expect
    .poll(async () =>
      page.evaluate(() =>
        (JSON.parse(document.getElementById('probe')!.dataset.slots ?? '[]') as { id: string }[])
          .map((slot) => slot.id)
          .sort(),
      ),
    )
    .toEqual([lone, group, review].sort());
});

test('proposal slots admit only bounded current unique geometry and cap convergence', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(() => {
    const Original = MessageChannel;
    Object.assign(window, {
      MessageChannel: class extends Original {
        constructor() {
          super();
          Object.assign(window, { slotPort: this.port1 });
          const post = this.port1.postMessage.bind(this.port1);
          this.port1.postMessage = (value: unknown) => {
            Object.assign(window, { slotRequest: value });
            post(value);
          };
        }
      },
    });
  });
  await mount(page, '<p>No placeholder</p>');
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  const result = await page.evaluate(() => {
    const w = window as unknown as {
      probe: { handle: { slots(v: { id: string; height: number }[]): void } };
      slotPort: MessagePort;
      slotRequest: { renderId: string; requestId: string; slots: { height: number }[] };
    };
    const id = '80000000-0000-4000-8000-000000000001';
    const reserve = () => w.probe.handle.slots([{ id, height: 99999 }]);
    reserve();
    const stale = w.slotRequest;
    reserve();
    const current = {
      type: 'colab.render.slots',
      renderId: w.slotRequest.renderId,
      requestId: w.slotRequest.requestId,
      slots: [{ id, top: 10 }],
    };
    const emit = (value: unknown) =>
      w.slotPort.dispatchEvent(new MessageEvent('message', { data: value }));
    const read = () => document.getElementById('probe')!.dataset.slots;
    const rejected = [];
    for (const value of [
      { ...current, renderId: 'old' },
      { ...current, requestId: stale.requestId },
      { ...current, slots: [{ id: 'foreign', top: 2 }] },
      { ...current, slots: [{ id, top: Infinity }] },
      { ...current, slots: [{ id, top: -1 }] },
      { ...current, slots: [current.slots[0], current.slots[0]] },
      { ...current, extra: true },
      {
        ...current,
        slots: [{ id, top: 10, beforeTop: 0, descriptionHeight: 10, left: 0, width: 100 }],
      },
      { ...current, slots: [{ id, height: 10 }] },
    ]) {
      emit(value);
      rejected.push(read());
    }
    const clamped = w.slotRequest.slots[0].height;
    for (const top of [10, 20, 30, 40]) emit({ ...current, slots: [{ id, top }] });
    const stopped = read();
    emit({ ...current, slots: [] });
    return { rejected, clamped, stopped, detached: read() };
  });
  expect(result.rejected).toEqual(Array(9).fill('[]'));
  expect(result.clamped).toBe(2000);
  expect(JSON.parse(result.stopped!)[0].top).toBe(30);
  expect(result.detached).toBe('[]');
});

test('selection mode outlines one target, captures text-bearing clicks and refuses textless targets', async ({
  page,
}) => {
  await page.goto('/');
  await mount(
    page,
    '<p id="target">Comment on these words</p><div id="empty" style="width:80px;height:80px"></div><p id="drag" style="display:inline-block">Drag these words</p>',
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  const frame = page.frames().find((frame) => frame.url().endsWith('/renderer.html'))!;
  await page.evaluate(() => {
    (
      window as unknown as { probe: { handle: { selectionMode(enabled: boolean): void } } }
    ).probe.handle.selectionMode(true);
  });
  await frame.locator('#target').hover();
  await expect(frame.locator('[data-colab-selection-outline]')).toHaveCount(1);
  expect(
    await frame
      .locator('[data-colab-selection-outline]')
      .evaluate((node) => getComputedStyle(node).backgroundColor),
  ).toBe('rgba(0, 0, 0, 0)');
  await frame.locator('#target').click();
  await expect(page.locator('#probe')).toHaveAttribute('data-selection', 'Comment on these words');
  await expect(page.locator('#probe')).toHaveAttribute('data-annotates', '1');
  const selected = JSON.parse((await page.locator('#probe').getAttribute('data-selector'))!);
  expect(selected.exact).toBe('Comment on these words');
  await frame.locator('#empty').click();
  await expect(frame.getByRole('status')).toHaveText('Select some text to comment');
  await expect(page.locator('#probe')).toHaveAttribute('data-annotates', '1');
  await frame.locator('#target').hover();
  await expect(frame.getByRole('status')).toHaveCount(0);
  const box = (await frame.locator('#drag').boundingBox())!;
  await page.mouse.move(box.x + 1, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width - 1, box.y + box.height / 2, { steps: 8 });
  await expect(page.locator('#probe')).toHaveAttribute('data-annotates', '1');
  await page.mouse.up();
  await expect(page.locator('#probe')).toHaveAttribute('data-selection', 'Drag these words');
  await expect(page.locator('#probe')).toHaveAttribute('data-annotates', '2');
  await page.evaluate(() => {
    (
      window as unknown as { probe: { handle: { selectionMode(enabled: boolean): void } } }
    ).probe.handle.selectionMode(false);
  });
  await expect(frame.locator('[data-colab-selection-outline]')).toHaveCount(0);
  await frame.locator('#target').click();
  await expect(page.locator('#probe')).toHaveAttribute('data-annotates', '2');
});

test('highlight clicks choose the shortest known quote while drag selection and scroll stay native', async ({
  page,
}) => {
  await page.goto('/');
  await mount(
    page,
    '<div style="height:1200px"></div><p style="font:20px monospace">Outer <span id="inner">words</span> tail</p><div style="height:1200px"></div>',
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  await page.evaluate(() => {
    const handle = (
      window as unknown as { probe: { handle: { highlight(value: unknown[]): void } } }
    ).probe.handle;
    handle.highlight([
      { id: 'outer', selector: { exact: 'Outer words tail', prefix: '', suffix: '' } },
      { id: 'inner-z', selector: { exact: 'words', prefix: 'Outer ', suffix: ' tail' } },
      { id: 'inner-a', selector: { exact: 'words', prefix: 'Outer ', suffix: ' tail' } },
    ]);
  });
  await expect(page.locator('#probe')).toHaveAttribute(
    'data-resolved',
    '["outer","inner-z","inner-a"]',
  );
  const frame = page.frameLocator('#probe iframe');
  const quote = frame.locator('#inner');
  await quote.scrollIntoViewIfNeeded();
  await expect
    .poll(async () => (await page.locator('#probe iframe').boundingBox())!.height)
    .toBeGreaterThan(2400);
  await quote.hover();
  await expect(frame.locator('[data-colab-selection-outline]')).toHaveCount(1);
  await expect(quote).toHaveCSS('cursor', 'pointer');
  const scroll = await page.evaluate(() => scrollY);
  const click = (await quote.boundingBox())!;
  await page.mouse.click(click.x + click.width / 2, click.y + click.height / 2);
  await expect(page.locator('#probe')).toHaveAttribute('data-opened', 'inner-a');
  expect(await page.evaluate(() => scrollY)).toBe(scroll);
  await page.evaluate(() => {
    delete document.querySelector<HTMLElement>('#probe')!.dataset.opened;
    (
      window as unknown as { probe: { handle: { selectionMode(value: boolean): void } } }
    ).probe.handle.selectionMode(true);
  });
  await quote.click({ modifiers: ['Control'] });
  await expect(page.locator('#probe')).not.toHaveAttribute('data-opened', /.+/);
  await page.clock.install();
  const hold = (await quote.boundingBox())!;
  await page.mouse.move(hold.x + hold.width / 2, hold.y + hold.height / 2);
  await page.mouse.down();
  await page.clock.fastForward(600);
  await page.mouse.up();
  await expect(page.locator('#probe')).not.toHaveAttribute('data-opened', /.+/);
  await quote.evaluate((node) => node.ownerDocument.getSelection()?.removeAllRanges());
  await quote.click();
  await expect(page.locator('#probe')).toHaveAttribute('data-opened', 'inner-a');
  await expect(page.locator('#probe')).not.toHaveAttribute('data-annotates', /.+/);
  await page.evaluate(() => {
    delete document.querySelector<HTMLElement>('#probe')!.dataset.opened;
  });
  const box = (await quote.boundingBox())!;
  await page.mouse.move(box.x + 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width - 2, box.y + box.height / 2, { steps: 6 });
  await page.mouse.up();
  await expect(page.locator('#probe')).toHaveAttribute('data-selection', /word/);
  await expect(page.locator('#probe')).toHaveAttribute('data-annotates', '1');
  await expect(page.locator('#probe')).not.toHaveAttribute('data-opened', /.+/);
  await quote.evaluate((node) => {
    node.ownerDocument.getSelection()?.removeAllRanges();
    const rect = node.getBoundingClientRect();
    node.dispatchEvent(
      new MouseEvent('click', {
        bubbles: true,
        clientX: rect.left + 2,
        clientY: rect.top + 2,
        detail: 1,
      }),
    );
  });
  await expect(page.locator('#probe')).not.toHaveAttribute('data-opened', /.+/);
  await page.evaluate(() => {
    const handle = (
      window as unknown as {
        probe: {
          handle: { highlight(value: unknown[]): void; selectionMode(value: boolean): void };
        };
      }
    ).probe.handle;
    handle.selectionMode(false);
    handle.highlight([]);
  });
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', '[]');
  await expect(frame.locator('[data-colab-selection-outline]')).toHaveCount(0);
  await quote.click();
  await expect(page.locator('#probe')).not.toHaveAttribute('data-opened', /.+/);
});

test.describe('touch highlight opening', () => {
  test.use({ hasTouch: true });
  test('a plain tap opens the existing known discussion', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto('/');
    await mount(page, '<p id="quote">Tap this existing highlight</p>');
    await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
    await page.evaluate(() => {
      (
        window as unknown as { probe: { handle: { highlight(value: unknown[]): void } } }
      ).probe.handle.highlight([
        {
          id: 'touch-thread',
          selector: { exact: 'Tap this existing highlight', prefix: '', suffix: '' },
        },
      ]);
    });
    await expect(page.locator('#probe')).toHaveAttribute('data-resolved', '["touch-thread"]');
    await page.locator('#probe iframe').evaluate((node) => {
      window.scrollTo({
        top: node.getBoundingClientRect().top + scrollY - 120,
        behavior: 'instant',
      });
    });
    const point = await page
      .frameLocator('#probe iframe')
      .locator('#quote')
      .evaluate((node) => {
        const range = node.ownerDocument.createRange();
        range.selectNodeContents(node);
        const box = range.getBoundingClientRect();
        return { x: box.left + box.width / 2, y: box.top + box.height / 2 };
      });
    const iframe = (await page.locator('#probe iframe').boundingBox())!;
    expect(iframe.y + point.y).toBeGreaterThan(56);
    expect(iframe.y + point.y).toBeLessThan(844);
    await page.touchscreen.tap(iframe.x + point.x, iframe.y + point.y);
    await expect(page.locator('#probe')).toHaveAttribute('data-opened', 'touch-thread');
    await expect(page.locator('#probe')).not.toHaveAttribute('data-annotates', /.+/);
  });
});

test('highlight hover coalesces trusted pointer bursts and cancels pending geometry', async ({
  page,
}) => {
  await page.goto('/');
  await mount(page, '<p id="quote" style="font:20px monospace">One highlighted passage</p>');
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  await page.evaluate(() => {
    (
      window as unknown as { probe: { handle: { highlight(value: unknown[]): void } } }
    ).probe.handle.highlight([
      { id: 'one', selector: { exact: 'One highlighted passage', prefix: '', suffix: '' } },
    ]);
  });
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', '["one"]');
  const frame = page.frameLocator('#probe iframe');
  const quote = frame.locator('#quote');
  await quote.scrollIntoViewIfNeeded();
  const box = (await quote.boundingBox())!;
  await page.clock.install({ time: new Date('2026-01-01T00:00:00Z') });
  await page.clock.pauseAt(new Date('2026-01-01T00:00:01Z'));
  // Drain pending size/marker placement before measuring the frozen pointer burst.
  await page.clock.runFor(200);
  await quote.evaluate(() => {
    const original = Range.prototype.getClientRects;
    let count = 0;
    Range.prototype.getClientRects = function () {
      if (
        this.toString() === 'One highlighted passage' &&
        new Error().stack?.includes('highlightAt')
      )
        count++;
      return original.call(this);
    };
    Object.assign(window, { hoverGeometryCount: () => count });
  });
  const reads = () =>
    quote.evaluate(() =>
      (window as unknown as { hoverGeometryCount(): number }).hoverGeometryCount(),
    );
  for (let offset = 0; offset < 12; offset++)
    await page.mouse.move(box.x + 10 + offset, box.y + box.height / 2);
  expect(await reads()).toBe(0);
  await expect(frame.locator('[data-colab-thread-outline]')).toHaveCount(0);
  await page.clock.runFor(20);
  expect(await reads()).toBe(1);
  await expect(frame.locator('[data-colab-thread-outline]')).toHaveAttribute(
    'data-colab-thread-outline',
    'one',
  );
  // The last point wins, even when the burst started on the highlighted text.
  await page.mouse.move(box.x + 12, box.y + box.height / 2);
  await page.mouse.move(box.x + box.width - 2, box.y + box.height / 2);
  await page.clock.runFor(20);
  expect(await reads()).toBe(2);
  await expect(frame.locator('[data-colab-thread-outline]')).toHaveCount(0);
  // A move out and back within the frozen frame must remain a drag, never a click.
  await page.mouse.move(box.x + 10, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + 100, box.y + box.height / 2);
  await page.mouse.move(box.x + 10, box.y + box.height / 2);
  await page.mouse.up();
  await expect(page.locator('#probe')).not.toHaveAttribute('data-opened', /.+/);
  await page.clock.runFor(20);
  expect(await reads()).toBe(3);
  await page.mouse.move(box.x + 11, box.y + box.height / 2);
  await quote.evaluate(() => dispatchEvent(new Event('scroll')));
  await page.clock.runFor(20);
  expect(await reads()).toBe(3);
  await expect(frame.locator('[data-colab-thread-outline]')).toHaveCount(0);
  await page.mouse.move(box.x + 12, box.y + box.height / 2);
  await quote.evaluate(() => dispatchEvent(new PageTransitionEvent('pagehide')));
  await page.clock.runFor(20);
  expect(await reads()).toBe(3);
});

test('a hovered thread passage is tinted through the highlight API, without an outline box', async ({
  page,
}) => {
  await page.goto('/');
  await mount(page, '<p id="quote" style="font:20px monospace">One highlighted passage</p>');
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  await page.evaluate(() => {
    (
      window as unknown as { probe: { handle: { highlight(value: unknown[]): void } } }
    ).probe.handle.highlight([
      { id: 'one', selector: { exact: 'One highlighted passage', prefix: '', suffix: '' } },
    ]);
  });
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', '["one"]');
  const frame = page.frameLocator('#probe iframe');
  const quote = frame.locator('#quote');
  const box = (await quote.boundingBox())!;
  const tinted = () => quote.evaluate(() => CSS.highlights.has('colab-hover'));
  expect(await tinted()).toBe(false);
  await page.mouse.move(box.x + 10, box.y + box.height / 2);
  await expect.poll(tinted).toBe(true);
  expect(await quote.evaluate(() => CSS.highlights.get('colab-hover')?.size)).toBe(1);
  const outline = frame.locator('[data-colab-thread-outline]');
  await expect(outline).toHaveAttribute('data-colab-thread-outline', 'one');
  expect(await outline.evaluate((node) => getComputedStyle(node).outlineStyle)).toBe('none');
  // Past the end of the line of text, still inside the frame: the pointer leaves the passage.
  await page.mouse.move(box.x + box.width - 2, box.y + box.height / 2);
  await expect.poll(tinted).toBe(false);
  await expect(outline).toHaveCount(0);
});

test('the rest highlight underlines the passage in the annotation rule colour', async ({
  page,
}) => {
  await page.goto('/');
  await mount(page, '<p id="quote">One highlighted passage</p>');
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  const frame = page.frameLocator('#probe iframe');
  const rules = await frame.locator('body').evaluate(() => {
    const read = (selector: string) =>
      [...document.styleSheets]
        .flatMap((sheet) => [...sheet.cssRules])
        .filter((rule): rule is CSSStyleRule => (rule as CSSStyleRule).selectorText === selector)
        .map((rule) => ({
          line: rule.style.getPropertyValue('text-decoration-line'),
          color: rule.style.getPropertyValue('text-decoration-color'),
          thickness: rule.style.getPropertyValue('text-decoration-thickness'),
          background: rule.style.getPropertyValue('background'),
        }));
    const root = getComputedStyle(document.documentElement);
    return {
      rest: read('::highlight(colab-comments)'),
      hover: read('::highlight(colab-hover)'),
      rule: root.getPropertyValue('--tmt-ui-color-annotation-rule').trim(),
      tint: root.getPropertyValue('--tmt-ui-surface-annotation-highlight').trim(),
      hoverTint: root.getPropertyValue('--tmt-ui-surface-annotation-highlight-hover').trim(),
    };
  });
  expect(rules.rule).toMatch(/^#[0-9a-f]{6}$/i);
  expect(rules.tint).toMatch(/^#[0-9a-f]{6}$/i);
  expect(rules.hoverTint).toMatch(/^#[0-9a-f]{6}$/i);
  expect(rules.rest).toEqual([
    {
      line: 'underline',
      color: 'var(--tmt-ui-color-annotation-rule)',
      thickness: '1px',
      background: 'var(--tmt-ui-surface-annotation-highlight)',
    },
  ]);
  expect(rules.hover).toEqual([
    {
      line: 'underline',
      color: 'var(--tmt-ui-color-annotation-rule)',
      thickness: '1px',
      background: 'var(--tmt-ui-surface-annotation-highlight-hover)',
    },
  ]);
});

for (const width of [1440, 390]) {
  test(`range-end markers keep a 24 px hit box around a 16 px circle without moving the text at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 844 });
    await page.goto('/');
    await mount(
      page,
      '<p id="quote" style="font:16px monospace">Alpha beta gamma delta epsilon zeta</p>',
    );
    await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
    const frame = page.frameLocator('#probe iframe');
    const quote = frame.locator('#quote');
    const before = await quote.evaluate((node) => ({
      rect: JSON.stringify(node.getBoundingClientRect()),
      scrollWidth: node.ownerDocument.documentElement.scrollWidth,
      scrollHeight: node.ownerDocument.documentElement.scrollHeight,
    }));
    await page.evaluate(() => {
      (
        window as unknown as { probe: { handle: { highlight(value: unknown[]): void } } }
      ).probe.handle.highlight(
        ['Alpha beta', 'Alpha beta', 'beta gamma', 'delta epsilon'].map((exact, index) => ({
          id: `t${index}`,
          selector: { exact, prefix: '', suffix: '' },
        })),
      );
    });
    const markers = frame.locator('[data-colab-thread]');
    await expect(markers).toHaveCount(4);
    // One synchronous snapshot: markers are replaced when the renderer re-places them.
    const placed = await markers.evaluateAll((nodes) =>
      nodes.map((node) => {
        const box = node.getBoundingClientRect();
        const style = getComputedStyle(node);
        return {
          x: box.x,
          y: box.y,
          width: box.width,
          height: box.height,
          viewport: innerWidth,
          padding: style.paddingTop,
          clip: style.backgroundClip,
          background: style.backgroundColor,
        };
      }),
    );
    expect(placed).toHaveLength(4);
    const boxes = placed.map(({ x, y, width: w, height }) => ({ x, y, width: w, height }));
    for (const marker of placed) {
      expect(marker).toMatchObject({ width: 24, height: 24, padding: '4px', clip: 'content-box' });
      expect(marker.background).not.toBe('rgba(0, 0, 0, 0)');
      expect(marker.x).toBeGreaterThanOrEqual(0);
      expect(marker.x + marker.width).toBeLessThanOrEqual(marker.viewport);
    }
    for (let first = 0; first < boxes.length; first++)
      for (let second = first + 1; second < boxes.length; second++) {
        const a = boxes[first]!;
        const b = boxes[second]!;
        const apart =
          a.x + a.width <= b.x ||
          b.x + b.width <= a.x ||
          a.y + a.height <= b.y ||
          b.y + b.height <= a.y;
        expect(apart, `hit boxes ${first} and ${second} overlap`).toBe(true);
      }
    const after = await quote.evaluate((node) => ({
      rect: JSON.stringify(node.getBoundingClientRect()),
      scrollWidth: node.ownerDocument.documentElement.scrollWidth,
      scrollHeight: node.ownerDocument.documentElement.scrollHeight,
    }));
    expect(after).toEqual(before);
  });
}

test('cosmetic key notifications admit only the current render and closed Escape/@ set', async ({
  page,
}) => {
  await page.goto('/');
  const snapshot = await mount(page, '<p id="quote" tabindex="0">Quote</p><input id="edit">');
  const probe = page.locator('#probe');
  await expect(probe).toHaveAttribute('data-state', 'ready');
  const frame = page.frames().find((frame) => frame.url().endsWith('/renderer.html'))!;
  const send = (value: unknown) => frame.evaluate((value) => parent.postMessage(value, '*'), value);
  for (const value of [
    { type: 'colab.render.key', renderId: snapshot.renderId, key: 'Enter' },
    { type: 'colab.render.key', renderId: 'old', key: 'Escape' },
    { type: 'colab.render.key', renderId: snapshot.renderId, key: 'Escape', extra: true },
  ])
    await send(value);
  // A later admitted message is an ordering fence for the preceding refusals.
  await send({ type: 'colab.render.key', renderId: snapshot.renderId, key: '@' });
  await expect(probe).toHaveAttribute('data-keys', '["@"]');
  await frame.locator('#quote').focus();
  for (const key of ['Shift+Escape', 'Alt+Escape', 'Control+Escape', 'Meta+Escape'])
    await page.keyboard.press(key);
  await page.keyboard.down('Escape');
  await page.keyboard.down('Escape');
  await page.keyboard.up('Escape');
  await expect(probe).toHaveAttribute('data-keys', '["@","Escape"]');
  await frame.locator('#edit').focus();
  await page.keyboard.press('Escape');
  await page.keyboard.press('@');
  const client = await page.context().newCDPSession(page);
  await frame.locator('#quote').focus();
  await client.send('Input.dispatchKeyEvent', {
    type: 'rawKeyDown',
    key: 'Escape',
    code: 'Escape',
    windowsVirtualKeyCode: 229,
  });
  await client.send('Input.dispatchKeyEvent', {
    type: 'keyUp',
    key: 'Escape',
    code: 'Escape',
    windowsVirtualKeyCode: 229,
  });
  await client.detach();
  await page.keyboard.press('@');
  await expect(probe).toHaveAttribute('data-keys', '["@","Escape","@"]');
});

test('reading headings resolve only requested IDs and jumps only highlight without opening a thread', async ({
  page,
}) => {
  const id = '10000000-0000-4000-8000-000000000401';
  await page.goto('/');
  await mount(
    page,
    `<tmt-read data-id="${id}" data-agent="Agent 1" data-why="Read this"><h2 data-title>Hosting</h2><p>The passage.</p></tmt-read>`,
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  await page.evaluate((id) => {
    const w = window as unknown as {
      probe: { handle: Awaited<ReturnType<typeof import('../src/renderer.js').mountRenderer>> };
    };
    w.probe.handle.questions([], [], [], [], [{ id, title: 'Hosting', closed: false }]);
  }, id);
  await expect.poll(() => page.locator('#probe').getAttribute('data-readings')).toContain(id);
  const readings = JSON.parse((await page.locator('#probe').getAttribute('data-readings'))!);
  expect(readings).toEqual([
    { id, selector: expect.objectContaining({ exact: 'Hosting' }), threads: [] },
  ]);
  await page.evaluate((id) => {
    const w = window as unknown as {
      probe: { handle: Awaited<ReturnType<typeof import('../src/renderer.js').mountRenderer>> };
    };
    w.probe.handle.scrollReading(id);
  }, id);
  const frame = page.frameLocator('#probe iframe');
  await expect
    .poll(() => frame.locator('body').evaluate(() => CSS.highlights.has('colab-reading')))
    .toBe(true);
  await expect(page.locator('#probe')).not.toHaveAttribute('data-opened');
});

test('reveal scrolls to the one element with an exact id and reports none or several', async ({
  page,
}) => {
  await page.goto('/');
  await mount(
    page,
    `<h1>Top</h1><div style="height:3000px">spacer</div><section id="decisions"><h2>Decisions</h2></section><p id="dup">a</p><p id="dup">b</p><div style="height:3000px">end</div>`,
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  const reveal = (id: string) =>
    page.evaluate((id) => {
      const w = window as unknown as {
        probe: { handle: { reveal(id: string): Promise<boolean> } };
      };
      return w.probe.handle.reveal(id);
    }, id);
  expect(await reveal('decisions')).toBe(true);
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBeGreaterThan(2500);
  const scrolled = await page.evaluate(() => window.scrollY);
  // Several matches, no match and an id outside the anchor grammar are all "not found", and
  // none of them moves the page.
  expect(await reveal('dup')).toBe(false);
  expect(await reveal('absent')).toBe(false);
  expect(await reveal('has space')).toBe(false);
  expect(await reveal('-leading')).toBe(false);
  expect(await page.evaluate(() => window.scrollY)).toBe(scrolled);
});

const frameHeight = (page: Page) =>
  page.locator('#probe iframe').evaluate((node) => node.clientHeight);

/** The target sits in the last screen of the page, so the host can never scroll all the way to it
 * (the page ends below it), and a spacer after it grows once, `delay` ms after the document loads. */
async function revealNearTheEnd(page: Page, delay: number) {
  await page.goto('/');
  await mount(
    page,
    `<style>@keyframes late{to{height:6000px}}.late{height:0;animation:late 10ms ${delay}ms forwards}</style><h1>Top</h1><div style="height:3000px">spacer</div><section id="decisions"><h2>Decisions</h2></section><p>tail</p><div class="late"></div>`,
  );
  await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
  // The frame has its first height, the spacer before the target included.
  await expect.poll(() => frameHeight(page)).toBeGreaterThan(3000);
  // Nothing below the frame: the page ends with it.
  await page.evaluate(() => {
    for (const node of document.querySelectorAll('body > :not(#probe)'))
      (node as HTMLElement).style.display = 'none';
  });
  const found = await page.evaluate(() => {
    const w = window as unknown as {
      probe: { handle: { reveal(id: string): Promise<boolean> } };
    };
    return w.probe.handle.reveal('decisions');
  });
  expect(found).toBe(true);
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBeGreaterThan(2500);
  return page.evaluate(() => window.scrollY);
}

test('a reveal finishes its scroll when the frame grows soon after it', async ({ page }) => {
  const reached = await revealNearTheEnd(page, 1200);
  await expect.poll(() => frameHeight(page)).toBeGreaterThan(8000);
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBeGreaterThan(reached + 100);
});

test('a frame growing long after a reveal does not pull the reader back to it', async ({
  page,
}) => {
  const reached = await revealNearTheEnd(page, 3500);
  await expect.poll(() => frameHeight(page)).toBeGreaterThan(8000);
  expect(await page.evaluate(() => window.scrollY)).toBe(reached);
});

test('a reader who scrolled after a reveal is not pulled back when the frame grows', async ({
  page,
}) => {
  const reached = await revealNearTheEnd(page, 1200);
  await page.evaluate(() => window.scrollBy(0, -500));
  await expect.poll(() => frameHeight(page)).toBeGreaterThan(8000);
  expect(await page.evaluate(() => window.scrollY)).toBe(reached - 500);
});

for (const admitted of [false, true])
  test(`an ${admitted ? 'admitted' : 'unadmitted'} reading tag cannot steal an existing proposal slot`, async ({
    page,
  }) => {
    const id = '10000000-0000-4000-8000-000000000401';
    await page.goto('/');
    await mount(
      page,
      `<tmt-proposal data-id="${id}"></tmt-proposal><tmt-read data-id="${id}" data-agent="Agent 1" data-why="Read this"><h2 data-title>Reading</h2></tmt-read>`,
    );
    await expect(page.locator('#probe')).toHaveAttribute('data-state', 'ready');
    await page.evaluate(
      ({ id, admitted }) => {
        const w = window as unknown as {
          probe: { handle: Awaited<ReturnType<typeof import('../src/renderer.js').mountRenderer>> };
        };
        w.probe.handle.questions(
          [],
          [],
          [],
          [],
          admitted ? [{ id, title: 'Reading', closed: false }] : [],
        );
        w.probe.handle.slots([{ id, height: 20 }]);
      },
      { id, admitted },
    );
    await expect.poll(() => page.locator('#probe').getAttribute('data-slots')).toContain(id);
    expect(JSON.parse((await page.locator('#probe').getAttribute('data-slots'))!)).toHaveLength(1);
  });
