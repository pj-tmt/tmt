import { pageAction } from '../test/page-actions.js';
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
      const host = document.createElement('div');
      host.id = 'probe';
      document.body.append(host);
      const controller = new AbortController();
      const handle = await mountRenderer(host, source, {
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
        onOpenThread: (id: string) => {
          host.dataset.opened = id;
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
  await expect(page.getByRole('heading', { name: 'My space' })).toBeVisible();
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
  await page.getByRole('link', { name: 'Space home', exact: true }).click();
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
  await expect(page.getByRole('heading', { name: 'My space' })).toBeVisible();
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
  await expect(page.getByRole('heading', { name: 'My space' })).toBeVisible();
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

test('range markers keep counts and quoted-text tooltips, follow document resize and scroll the window to their anchor', async ({
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
  const marker = frame.locator('[data-colab-thread]');
  await expect(marker).toHaveCount(1);
  await expect(marker).toHaveText('2');
  await expect(marker).toHaveAttribute('title', 'Anchored line');
  const alignment = await frame.locator('#quote').evaluate((node) => {
    const range = node.ownerDocument.createRange();
    range.selectNodeContents(node);
    return Math.abs(
      node.ownerDocument.querySelector('[data-colab-thread]')!.getBoundingClientRect().top -
        range.getBoundingClientRect().top,
    );
  });
  expect(alignment).toBeLessThanOrEqual(5);
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
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(marker).toHaveCount(1);
  await frame.locator('#quote').evaluate((node) => {
    node.textContent = 'Changed source';
  });
  await expect(marker).toHaveCount(0);
  await expect(page.locator('#probe')).toHaveAttribute('data-resolved', '[]');
});

test('only IDs and quote selectors reach author code even when a caller supplies private comment fields', async ({
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
    { id: 'known', selector: { exact: 'Anchor quote', prefix: '', suffix: '' } },
  ]);
  expect(observed).not.toContain(secret);
  await expect(frame.locator('[data-colab-thread]')).toHaveAttribute('title', 'Anchor quote');
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

test('keyboard selection waits for key release; C and Alt+Enter keep the embed key guards', async ({
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
  expect(
    await frame.evaluate(() => (window as unknown as { confirmationKey: unknown }).confirmationKey),
  ).toEqual({ trusted: true, code: 'KeyC', keyCode: 229 });
  await client.detach();
  await page.keyboard.press('c');
  await expect(page.locator('#probe')).toHaveAttribute('data-annotates', '1');
  await page.keyboard.down('c');
  await expect(page.locator('#probe')).toHaveAttribute('data-annotates', '2');
  await page.keyboard.down('c');
  await page.keyboard.up('c');
  await expect(page.locator('#probe')).toHaveAttribute('data-annotates', '2');
  await page.keyboard.press('Alt+Enter');
  await expect(page.locator('#probe')).toHaveAttribute('data-annotates', '3');
  for (const id of ['edit', 'area', 'editable']) {
    await frame.locator(`#${id}`).focus();
    await page.keyboard.press('c');
    await page.keyboard.press('Alt+Enter');
    await expect(page.locator('#probe')).toHaveAttribute('data-annotates', '3');
  }
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
  expect(result.rejected).toEqual(Array(7).fill('[]'));
  expect(result.clamped).toBe(2000);
  expect(JSON.parse(result.stopped!)[0].top).toBe(30);
  expect(result.detached).toBe('[]');
});
