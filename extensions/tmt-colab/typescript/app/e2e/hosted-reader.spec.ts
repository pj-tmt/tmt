import { text } from '../src/strings.js';
import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
import http from 'node:http';
import { finiteLoopbackResponse } from '../acceptance/harness/firestore-proxy.js';

test.use({ timezoneId: 'Asia/Tokyo' });

const origin = 'https://demo-tmt-hosted.web.app';
async function hostedFixture(page: Page, baseURL: string | undefined): Promise<string[]> {
  const outside: string[] = [];
  const lifetime = new AbortController();
  page.once('close', () => lifetime.abort());
  await page.routeWebSocket('**/*', (socket) => socket.close());
  await page.route('**/*', async (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== origin) {
      outside.push(url.origin);
      await route.abort();
    } else {
      const request = route.request();
      const headers = await request.allHeaders();
      // The transport target is the private dev server, not the browser's logical origin.
      delete headers.host;
      try {
        await route.fulfill(
          await finiteLoopbackResponse({
            url: new URL(`${baseURL}${url.pathname}${url.search}`),
            method: request.method(),
            headers,
            body: request.postDataBuffer(),
            signal: lifetime.signal,
          }),
        );
      } catch (error) {
        await route.abort();
        throw error;
      }
    }
  });
  return outside;
}

async function fixtureUpstream(handler: http.RequestListener) {
  const server = http.createServer(handler);
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const { port } = server.address() as { port: number };
  return {
    url: `http://127.0.0.1:${port}`,
    async close() {
      server.closeAllConnections();
      await new Promise<void>((resolve, reject) =>
        server.close((error) => (error ? reject(error) : resolve())),
      );
    },
  };
}

test('hosted module proxy uses a fresh upstream connection for each response', async ({ page }) => {
  const sockets = new WeakSet<object>();
  let requests = 0;
  const upstream = await fixtureUpstream((request, response) => {
    requests++;
    if (sockets.has(request.socket)) {
      request.socket.destroy();
      return;
    }
    sockets.add(request.socket);
    response.setHeader('content-type', 'text/html');
    response.end('<h1>Original fixture</h1>');
  });
  try {
    const outside = await hostedFixture(page, upstream.url);
    for (const path of ['/first', '/next']) {
      await page.goto(origin + path);
      await expect(page.getByRole('heading', { name: 'Original fixture' })).toBeVisible();
    }
    expect(requests).toBe(2);
    expect(outside).toEqual([]);
  } finally {
    await page.close();
    await upstream.close();
  }
});

test('hosted module proxy preserves upstream refusals, headers and binary bytes', async ({
  page,
}) => {
  const bytes = [0, 255, 1, 128];
  const received: Buffer[] = [];
  let method: string | undefined;
  let proof: string | string[] | undefined;
  const upstream = await fixtureUpstream((request, response) => {
    if (request.url === '/start') {
      response.setHeader('content-type', 'text/html');
      response.end('<h1>Proxy control</h1>');
      return;
    }
    method = request.method;
    proof = request.headers['x-proof'];
    request.on('data', (part: Buffer) => received.push(part));
    request.on('end', () => {
      response.writeHead(409, {
        'content-type': 'application/octet-stream',
        'x-proof': 'original refusal',
      });
      response.end(Buffer.from(bytes));
    });
  });
  try {
    const outside = await hostedFixture(page, upstream.url);
    await page.goto(`${origin}/start`);
    const reply = await page.evaluate(async (bytes) => {
      const response = await fetch('/binary', {
        method: 'POST',
        headers: { 'x-proof': 'original request' },
        body: new Uint8Array(bytes),
      });
      return {
        status: response.status,
        type: response.headers.get('content-type'),
        proof: response.headers.get('x-proof'),
        bytes: [...new Uint8Array(await response.arrayBuffer())],
      };
    }, bytes);
    expect(reply).toEqual({
      status: 409,
      type: 'application/octet-stream',
      proof: 'original refusal',
      bytes,
    });
    expect(method).toBe('POST');
    expect(proof).toBe('original request');
    expect(Buffer.concat(received)).toEqual(Buffer.from(bytes));
    expect(outside).toEqual([]);
  } finally {
    await page.close();
    await upstream.close();
  }
});

// This fixture verifies actual chrome/rendering, not authority or provider behavior. The embedded
// hosted-bundle acceptance separately enforces the composed CSP with the real SDK bytes.
for (const width of [390, 1280]) {
  test(`hosted read-only states and renderer at ${width}px`, async ({ page, baseURL }) => {
    const outside = await hostedFixture(page, baseURL);
    await page.setViewportSize({ width, height: 844 });
    await page.clock.setFixedTime(new Date('2026-11-02T00:00:00Z'));
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    for (const state of [
      'sign-in',
      'verifying',
      'recovering',
      'paused',
      'ready',
      'reshare',
      'budget',
      'rate-limited',
      'denied',
      'missing',
    ]) {
      await page.goto(`${origin}/test/hosted-browser.html?state=${state}`);
      if (state === 'ready') {
        await expect(
          page.frameLocator('iframe').getByRole('heading', { name: 'Verified shared page' }),
        ).toBeVisible();
        await expect(page.locator('iframe')).toHaveAttribute('src', `${origin}/renderer.html`);
        await expect(page.getByText('Live', { exact: true })).toBeVisible();
        await expect(
          page.getByText('This browser is close to its daily reading allowance.', {
            exact: false,
          }),
        ).toBeVisible();
        await expect(page.getByRole('button', { name: text.save, exact: true })).toHaveCount(0);
        await expect(page.getByRole('button', { name: /^(Ask|Share)$/ })).toHaveCount(0);
        await page.getByLabel('Info', { exact: true }).click();
        const panel = page.locator('.reader-information-panel');
        await expect(panel).toBeVisible();
        const box = (await panel.boundingBox())!;
        expect(box.x).toBeGreaterThanOrEqual(0);
        expect(box.x + box.width).toBeLessThanOrEqual(width);
        await page.getByLabel('Info', { exact: true }).click();
      } else {
        await expect(page.getByTestId('hosted-entry')).toBeVisible();
        await expect(page.locator('iframe')).toHaveCount(0);
        await expect(page.getByText('Live', { exact: true })).toHaveCount(0);
        if (state === 'sign-in')
          await expect(page.getByRole('button', { name: 'Sign in', exact: true })).toBeVisible();
        if (state === 'reshare')
          await expect(
            page.getByText(
              'This page changed its sharing since this link was made. Ask the owner to share it again.',
            ),
          ).toBeVisible();
        if (state === 'budget') {
          await expect(page.getByText('The allowance resets at 17:00.')).toBeVisible();
          await expect(page.getByText('Limit reached', { exact: true })).toBeVisible();
          await expect(page.getByText('failed', { exact: true })).toHaveCount(0);
          await expect(page.getByRole('button')).toHaveCount(0);
        }
      }
      if (state === 'rate-limited') {
        await expect(
          page
            .getByTestId('hosted-entry')
            .getByRole('heading', { name: 'Too many requests', exact: true }),
        ).toBeVisible();
        await expect(page.getByText('Paused', { exact: true })).toBeVisible();
        await expect(page.getByText('failed', { exact: true })).toHaveCount(0);
        await expect(page.getByRole('button')).toHaveCount(0);
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        width,
      );
      await page.screenshot({ path: capturePath(`hosted-${state}-${width}.png`), fullPage: true });
    }
    expect(errors).toEqual([]);
    expect(outside).toEqual([]);
  });
}

for (const width of [390, 1280]) {
  for (const colorScheme of ['light', 'dark'] as const) {
    test(`hosted role and outcome copy at ${width}px ${colorScheme}`, async ({ page, baseURL }) => {
      const outside = await hostedFixture(page, baseURL);
      await page.setViewportSize({ width, height: 844 });
      await page.emulateMedia({ colorScheme });
      await page.clock.setFixedTime(new Date('2026-11-02T00:00:00Z'));
      const errors: string[] = [];
      page.on('pageerror', (error) => errors.push(error.message));
      for (const state of [
        'sign-in-viewer',
        'sign-in-commenter',
        'sign-in-editor',
        'editor',
        'request-waiting',
        'request-changed',
        'request-large',
        'request-refused',
        'request-unconfirmed',
        'budget',
      ]) {
        await page.goto(`${origin}/test/hosted-browser.html?state=${state}`);
        if (state === 'editor' || state.startsWith('request-')) {
          await expect(
            page
              .frameLocator('iframe')
              .getByRole('heading', { name: 'Verified shared page', exact: true }),
          ).toBeVisible();
          await page.getByTestId('source-toggle').click();
          const area = page.locator('#source-edit');
          await expect(area).toBeVisible();
          if (state.startsWith('request-')) {
            await area.fill('<main><h1>Requested draft</h1></main>');
            await page.getByRole('button', { name: text.save, exact: true }).click();
            // A sent request is answered by the page: the same copy as a native link's.
            await expect(
              page.getByText(
                {
                  'request-waiting': text.linkEditWaiting,
                  'request-changed': text.linkEditChanged,
                  'request-large': text.linkEditTooLarge(200 * 1024, 128 * 1024),
                  'request-refused': 'Not sent',
                  'request-unconfirmed': 'Unconfirmed',
                }[state]!,
                { exact: true },
              ),
            ).toBeVisible();
            await expect(area).toHaveValue('<main><h1>Requested draft</h1></main>');
          }
        }
        if (state === 'request-unconfirmed') {
          await expect(page.getByText('Unconfirmed', { exact: true })).toBeVisible();
          await expect(
            page.getByText(
              "Couldn't confirm the edit request. It may still reach the owner; check the page before sending it again.",
              { exact: true },
            ),
          ).toBeVisible();
          await expect(page.locator('[data-tone="waiting"]')).toBeVisible();
          await expect(page.getByText('Not sent', { exact: true })).toHaveCount(0);
        } else if (state.startsWith('sign-in-')) {
          await expect(page.getByRole('button', { name: 'Sign in', exact: true })).toBeVisible();
          await expect(
            page.getByText(state === 'sign-in-editor' ? 'Can edit' : 'Read-only', { exact: true }),
          ).toBeVisible();
          await expect(
            page.getByText(
              state === 'sign-in-editor'
                ? 'Sign in to read this page and send edit requests. The owner applies edits while Colab is running.'
                : 'Sign in to read the shared page. Signing in gives read access only; you cannot edit the page.',
              { exact: true },
            ),
          ).toBeVisible();
          await expect(page.getByText('Can comment', { exact: true })).toHaveCount(0);
        }
        expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
          width,
        );
        await page.screenshot({
          path: capturePath(`hosted-${state}-${width}-${colorScheme}.png`),
          fullPage: true,
        });
      }
      expect(errors).toEqual([]);
      expect(outside).toEqual([]);
    });
  }
}
