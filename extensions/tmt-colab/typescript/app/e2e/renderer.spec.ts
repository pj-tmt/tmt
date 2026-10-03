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
        onSelection: (text: string) => {
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
  await expect(page.locator('.status').filter({ hasText: 'Live preview' })).toBeVisible();
  const frame = page.frameLocator('iframe');
  await expect(frame.getByRole('heading', { name: 'Small pages, shared ideas.' })).toBeVisible();
  await frame.getByRole('button', { name: 'Try the page: 0' }).click();
  await expect(frame.getByRole('button', { name: 'Try the page: 1' })).toBeVisible();
  await expect(page.locator('iframe')).toHaveAttribute('sandbox', 'allow-scripts');
  await expect(page.locator('iframe')).toHaveAttribute('referrerpolicy', 'no-referrer');
  await expect(page.locator('iframe')).not.toHaveAttribute('srcdoc');
  await expect(page.locator('iframe')).toHaveAttribute('src', /\/renderer\.html$/);
  await page.screenshot({ path: '/tmp/1187-page-view.png', fullPage: true });
  await page.getByRole('button', { name: 'Source', exact: true }).click();
  await expect(page.getByRole('textbox')).toHaveAttribute('readonly', '');
  await page.screenshot({ path: '/tmp/1187-page-light.png', fullPage: true });
  const renderId = await page.locator('iframe').getAttribute('data-render-id');
  await page.getByRole('button', { name: 'Change color theme' }).click();
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
    // The parent receives all three messages in order; valid final text proves
    // the channel remains live after both rejected inputs.
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
