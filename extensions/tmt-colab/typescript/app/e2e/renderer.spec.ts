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

async function mount(page: Page, source: string, probeAdmission = false) {
  return page.evaluate(
    async ({ source, probeAdmission }) => {
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
      });
      // Test-owned handles never exist in the production entry point.
      Object.assign(window, { probe: { handle, controller } });
      const admission: (string | undefined)[] = [];
      if (probeAdmission) {
        // Run in the same task as insertion, before the genuine frame load/reply.
        const frame = host.querySelector('iframe')!;
        const reply = (source: Window, renderId: string) => {
          window.dispatchEvent(
            new MessageEvent('message', {
              source,
              data: { type: 'colab.render.bound', renderId },
            }),
          );
          admission.push(host.dataset.state);
        };
        reply(window, handle.snapshot.renderId);
        reply(frame.contentWindow!, 'stale-id');
        reply(frame.contentWindow!, handle.snapshot.renderId);
      }
      return { ...handle.snapshot, admission } as {
        renderId: string;
        sourceDigest: string;
        source: string;
        admission: (string | undefined)[];
      };
    },
    { source, probeAdmission },
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
  await page.getByRole('button', { name: 'Source', exact: true }).click();
  await expect(page.getByRole('textbox')).toHaveAttribute('readonly', '');
  await page.screenshot({ path: '/tmp/1187-page-light.png', fullPage: true });
  const renderId = await page.locator('iframe').getAttribute('data-render-id');
  await page.getByRole('button', { name: 'Change color theme' }).click();
  await expect(frame.getByRole('button', { name: 'Try the page: 1' })).toBeVisible();
  await expect(page.locator('iframe')).toHaveAttribute('data-render-id', renderId!);
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
      popup: window.open('${origin}/popup') === null,
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
  const source = '<p>First</p>';
  const captured = await mount(page, source, true);
  expect(captured.admission).toEqual([undefined, undefined, 'ready']);
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
