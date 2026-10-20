import { readFileSync } from 'node:fs';
import { mkdirSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { expect, test, type Page } from '@playwright/test';

// Use the shipped author starter rather than maintaining another palette fixture.
const skill = readFileSync(
  new URL('../../../skills/tmt-colab/references/page-style.md', import.meta.url),
  'utf8',
);
const style = skill.match(/<style>[\s\S]*?<\/style>/)?.[0];
if (!style) throw new Error('The bundled author starter is missing.');
const content = `<h1>Review notes</h1>
<p class="muted">A page begins with its content. Colab supplies the title and actions above.</p>
<section><h2>Release readiness</h2><p>◆ Waiting — review the candidate before release.</p><ul><li>Verify the changed behaviour.</li><li>Record the reviewed head and evidence.</li></ul></section>
<h2 id="checks">Checks</h2><table><thead><tr><th>Check</th><th>Result</th></tr></thead><tbody><tr><td>Token equality</td><td>✓ Passed</td></tr><tr><td>Narrow layout</td><td>✓ Passed</td></tr></tbody></table>
<h2>Measured usage</h2><div id="sample-chart"><svg viewBox="0 0 320 64" role="img" aria-label="Sample usage chart" style="display:block;width:100%;height:80px"><rect x="0" y="32" width="96" height="32" fill="currentColor"/><rect x="112" y="16" width="96" height="48" fill="currentColor"/><rect x="224" y="0" width="96" height="64" fill="currentColor"/></svg></div>
<h2>Next step</h2><p>Copy the inline starter into your page and write semantic content.</p><pre><code>tmt colab skill</code></pre><p><a href="#checks">Read the checks</a> and record the result.</p>`;
const sample = `<!doctype html><html><head><meta charset="utf-8">${style}</head><body>${content}</body></html>`;
const interactive = `${sample}<button onclick="this.textContent='Count: '+(++window.count)">Count: 0</button>
<script>window.count=0;document.body.dataset.initialTheme=document.documentElement.dataset.theme;</script>`;
const captures = process.env.COLAB_THEME_CAPTURE_DIR;
if (captures) mkdirSync(captures, { recursive: true });

async function mount(page: Page, source: string, reader = false) {
  await page.goto('/');
  await page.evaluate(
    async ({ source, reader }) => {
      const path = '/test/page-layout-browser.tsx';
      const fixture = await import(path);
      await fixture[reader ? 'mountReader' : 'mount'](source);
    },
    { source, reader },
  );
  await expect(page.locator('#layout-fixture .status')).toContainText('Live');
  await expect(
    page.frameLocator('#layout-fixture iframe').getByRole('heading', { name: 'Review notes' }),
  ).toBeVisible();
}

async function palette(page: Page, theme: 'light' | 'dark') {
  const frame = page.frameLocator('#layout-fixture iframe');
  await expect(frame.locator('body')).toHaveCSS(
    'background-color',
    theme === 'dark' ? 'rgb(8, 8, 8)' : 'rgb(250, 250, 250)',
  );
  await expect(frame.locator('body')).toHaveCSS(
    'color',
    theme === 'dark' ? 'rgb(176, 176, 176)' : 'rgb(52, 52, 52)',
  );
  await expect(frame.locator('html')).toHaveAttribute('data-theme', theme);
  await expect(frame.locator('html')).toHaveCSS('color-scheme', theme);
}

async function choose(page: Page, theme: 'light' | 'dark') {
  if ((await page.evaluate(() => document.documentElement.dataset.theme)) === theme) return;
  await page.getByRole('button', { name: 'More', exact: true }).click();
  await page
    .getByRole('menuitemradio', {
      name: `Theme: ${theme === 'dark' ? 'Dark' : 'Light'}`,
      exact: true,
    })
    .click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
}

for (const width of [1280, 1440, 390]) {
  test(`starter container ${width}: centred cap and shared content width`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ colorScheme: 'light' });
    await mount(page, sample);
    const child = page.frameLocator('#layout-fixture iframe');
    const geometry = await child.locator('body').evaluate((body) => {
      const bounds = body.getBoundingClientRect();
      const css = getComputedStyle(body);
      const gutter = parseFloat(css.paddingLeft);
      const content = ['p', 'table', '#sample-chart', 'pre'].map((selector) => {
        const box = body.querySelector(selector)!.getBoundingClientRect();
        return { selector, x: box.x, width: box.width };
      });
      return {
        viewport: innerWidth,
        x: bounds.x,
        width: bounds.width,
        gutter,
        scrollWidth: document.documentElement.scrollWidth,
        content,
      };
    });
    expect(geometry.viewport).toBe(width);
    expect(geometry.width).toBe(Math.min(width, 1280));
    expect(geometry.x).toBe((width - geometry.width) / 2);
    expect(geometry.gutter).toBe(16);
    expect(geometry.scrollWidth).toBe(width);
    for (const block of geometry.content) {
      expect(block.x, block.selector).toBe(geometry.x + geometry.gutter);
      expect(block.width, block.selector).toBe(geometry.width - 2 * geometry.gutter);
    }
    if (captures && width !== 1280) {
      await page.screenshot({ path: `${captures}/container-${width}.png`, fullPage: true });
    }
  });
}

for (const width of [1440, 390]) {
  for (const os of ['light', 'dark'] as const) {
    for (const theme of ['light', 'dark'] as const) {
      test(`sample ${width}: OS ${os}, Colab ${theme}, live choice without reload`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.emulateMedia({ colorScheme: os });
        await mount(page, interactive);
        const frame = page.locator('#layout-fixture iframe');
        const child = page.frameLocator('#layout-fixture iframe');
        const renderId = await frame.getAttribute('data-render-id');
        const digest = await frame.getAttribute('data-source-digest');
        expect(digest).toBe(createHash('sha256').update(interactive).digest('hex'));
        await child.getByRole('button', { name: 'Count: 0' }).click();
        await choose(page, theme);
        await palette(page, theme);
        await expect(frame).toHaveAttribute('data-render-id', renderId!);
        await expect(frame).toHaveAttribute('data-source-digest', digest!);
        await expect(child.getByRole('button', { name: 'Count: 1' })).toBeVisible();
        await expect(child.locator('body')).toHaveAttribute('data-initial-theme', os);
        expect(
          await child
            .locator('html')
            .evaluate(() => matchMedia('(prefers-color-scheme: dark)').matches),
        ).toBe(os === 'dark');
        // Capture the exact content-only #2225 sample; remove only test instrumentation.
        await child.getByRole('button', { name: 'Count: 1' }).evaluate((node) => node.remove());
        if (captures) await page.mouse.move(width - 1, 899);
        if (captures)
          await page.screenshot({
            path: `${captures}/sample-${width}-os-${os}-colab-${theme}.png`,
          });
        await page.emulateMedia({ colorScheme: os === 'dark' ? 'light' : 'dark' });
        await palette(page, theme);
        await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
        await page
          .locator('#layout-fixture')
          .getByRole('link', { name: /^Colab · local/ })
          .click();
        await expect(frame).toHaveCount(0);
        await page
          .locator('#layout-fixture')
          .getByRole('link', { name: 'Release notes', exact: true })
          .click();
        await palette(page, theme);
        await expect(child.locator('body')).toHaveAttribute('data-initial-theme', theme);
        await expect(frame).not.toHaveAttribute('data-render-id', renderId!);
      });
    }
  }
}

for (const reader of [false, true]) {
  test(`${reader ? 'reader' : 'owner'}: no choice follows live OS preference`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: 'light' });
    await mount(page, interactive, reader);
    const frame = page.locator('#layout-fixture iframe');
    const renderId = await frame.getAttribute('data-render-id');
    await expect(page.locator('html')).not.toHaveAttribute('data-theme');
    await palette(page, 'light');
    await page.emulateMedia({ colorScheme: 'dark' });
    await palette(page, 'dark');
    await expect(page.locator('html')).not.toHaveAttribute('data-theme');
    await expect(frame).toHaveAttribute('data-render-id', renderId!);
    await page.emulateMedia({ colorScheme: 'light' });
    await palette(page, 'light');
    if (reader)
      await expect(
        page.locator('#layout-fixture').getByRole('button', { name: 'Change color theme' }),
      ).toHaveCount(0);
  });
}

test('media-only author CSS keeps the OS palette despite Colab choice', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 900 });
  await page.emulateMedia({ colorScheme: 'light' });
  await mount(
    page,
    `<style>:root{color-scheme:light dark}body{background:#fafafa;color:#343434;font:14px system-ui}@media(prefers-color-scheme:dark){body{background:#080808;color:#b0b0b0}}</style><h1>Review notes</h1><p>Media-only CSS follows the OS. Use the starter's data-theme selectors to follow Colab's choice.</p>`,
  );
  await choose(page, 'dark');
  const child = page.frameLocator('#layout-fixture iframe');
  await expect(child.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(child.locator('body')).toHaveCSS('background-color', 'rgb(250, 250, 250)');
  if (captures)
    await page.screenshot({ path: `${captures}/media-only-390-os-light-colab-dark.png` });
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(child.locator('body')).toHaveCSS('background-color', 'rgb(8, 8, 8)');
});

test('only exact current-render theme messages on the bound port update author presentation', async ({
  page,
}) => {
  await page.goto('/');
  const renderId = `00000000-0000-4000-8000-000000000001:${'a'.repeat(64)}`;
  await page.evaluate(
    async ({ source, renderId }) => {
      const frame = document.createElement('iframe');
      frame.id = 'theme-target';
      frame.setAttribute('sandbox', 'allow-scripts');
      const loaded = new Promise<void>((resolve) => {
        frame.onload = () => resolve();
      });
      frame.src = '/renderer.html';
      document.body.append(frame);
      await loaded;
      const channel = new MessageChannel();
      Object.assign(window, { themePort: channel.port1 });
      frame.contentWindow!.postMessage(
        {
          type: 'colab.render.bind',
          source,
          sourceDigest: 'a'.repeat(64),
          renderId,
          theme: 'light',
        },
        '*',
        [channel.port2],
      );
    },
    { source: sample, renderId },
  );
  const child = page.frameLocator('#theme-target');
  await expect(child.locator('html')).toHaveAttribute('data-theme', 'light');
  await child.locator('html').evaluate((node) => {
    const changes: string[] = [];
    new MutationObserver((records) =>
      records.forEach(() => changes.push(node.getAttribute('data-theme')!)),
    ).observe(node, { attributes: true, attributeFilter: ['data-theme'] });
    Object.assign(window, { themeChanges: changes });
  });
  await page.evaluate(async (renderId) => {
    const port = (window as unknown as { themePort: MessagePort }).themePort;
    const valid = { type: 'colab.render.theme', renderId, theme: 'dark' };
    for (const value of [
      null,
      [],
      { ...valid, renderId: 'stale' },
      { ...valid, extra: true },
      { ...valid, theme: 'auto' },
      { ...valid, theme: null },
    ])
      port.postMessage(value);
    document
      .querySelector<HTMLIFrameElement>('#theme-target')!
      .contentWindow!.postMessage(valid, '*');
    port.postMessage(valid);
    // The existing highlight response is a barrier after all prior port traffic.
    const requestId = '00000000-0000-4000-8000-000000000002';
    const finished = new Promise<void>((resolve) => {
      port.onmessage = (event) => {
        if (event.data.type === 'colab.render.anchors' && event.data.requestId === requestId)
          resolve();
      };
    });
    port.postMessage({
      type: 'colab.render.highlight',
      renderId,
      requestId,
      anchors: [],
      threadCount: 0,
      draft: null,
    });
    await finished;
  }, renderId);
  await expect(child.locator('html')).toHaveAttribute('data-theme', 'dark');
  expect(
    await child
      .locator('html')
      .evaluate(() => (window as unknown as { themeChanges: string[] }).themeChanges),
  ).toEqual(['dark']);
  await page.evaluate(() => {
    (window as unknown as { themePort: MessagePort }).themePort.close();
    document.querySelector('#theme-target')!.remove();
  });
});

test('author-to-parent theme claims cannot change the chrome choice', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'light' });
  await mount(page, sample);
  await choose(page, 'light');
  await page.evaluate(() => {
    const receive = (event: MessageEvent) => {
      const frame = document.querySelector<HTMLIFrameElement>('#layout-fixture iframe');
      if (event.source !== frame?.contentWindow || event.data?.type !== 'theme-test.barrier')
        return;
      document.documentElement.dataset.themeClaimSeen = 'true';
      removeEventListener('message', receive);
    };
    addEventListener('message', receive);
  });
  const renderId = await page.locator('#layout-fixture iframe').getAttribute('data-render-id');
  await page
    .frameLocator('#layout-fixture iframe')
    .locator('html')
    .evaluate(async (node, renderId) => {
      parent.postMessage({ type: 'colab.render.theme', renderId, theme: 'dark' }, '*');
      node.ownerDocument.defaultView!.parent.postMessage({ type: 'theme-test.barrier' }, '*');
    }, renderId);
  await expect(page.locator('html')).toHaveAttribute('data-theme-claim-seen', 'true');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await palette(page, 'light');
});

test('a theme change between source init and acknowledgement reaches the same renderer', async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: 'light' });
  await page.goto('/');
  await page.evaluate(() => {
    const changed = (event: MessageEvent) => {
      if (event.data?.type !== 'theme-test.loading') return;
      document.documentElement.dataset.theme = 'dark';
      removeEventListener('message', changed);
    };
    addEventListener('message', changed);
  });
  await page.evaluate(async (source) => {
    const path = '/test/page-layout-browser.tsx';
    (await import(path)).mount(source);
  }, `${interactive}<script>parent.postMessage({type:'theme-test.loading'},'*')</script>`);
  await expect(page.locator('#layout-fixture .status')).toContainText('Live');
  await expect(page.frameLocator('#layout-fixture iframe').locator('body')).toHaveAttribute(
    'data-initial-theme',
    'light',
  );
  await palette(page, 'dark');
});

test('render release, abort, destroy and navigation remove theme subscriptions, and a renderer that never answers stays loading until it is left', async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: 'light' });
  await page.goto('/');
  await page.evaluate(() => {
    // Count only subscriptions owned by mounts made after this instrumentation.
    const NativeObserver = MutationObserver;
    const observers = new Set<MutationObserver>();
    const listeners = new Set<EventListenerOrEventListenerObject>();
    window.MutationObserver = class extends NativeObserver {
      observe(target: Node, options?: MutationObserverInit) {
        if (
          target === document.documentElement &&
          options?.attributeFilter?.join() === 'data-theme'
        )
          observers.add(this);
        super.observe(target, options);
      }
      disconnect() {
        observers.delete(this);
        super.disconnect();
      }
    };
    const nativeMedia = matchMedia.bind(window);
    window.matchMedia = (query) => {
      const media = nativeMedia(query);
      if (query === '(prefers-color-scheme: dark)') {
        const add = media.addEventListener.bind(media),
          remove = media.removeEventListener.bind(media);
        media.addEventListener = (
          type: string,
          listener: EventListenerOrEventListenerObject,
          options?: boolean | AddEventListenerOptions,
        ) => {
          if (type === 'change') listeners.add(listener);
          add(type, listener, options);
        };
        media.removeEventListener = (
          type: string,
          listener: EventListenerOrEventListenerObject,
          options?: boolean | EventListenerOptions,
        ) => {
          if (type === 'change') listeners.delete(listener);
          remove(type, listener, options);
        };
      }
      return media;
    };
    Object.assign(window, {
      themeSubscriptions: () => ({ observers: observers.size, media: listeners.size }),
    });
  });
  const mountProbe = async (source: string) => {
    await page.evaluate(async (source) => {
      const path = '/src/renderer.ts';
      const { mountRenderer } = await import(path);
      let host = document.getElementById('theme-probe');
      if (!host) {
        host = document.createElement('div');
        host.id = 'theme-probe';
        document.body.append(host);
      }
      const controller = new AbortController();
      const handle = await mountRenderer(host, source, {
        signal: controller.signal,
        onState: (state: string) => {
          host!.dataset.state = state;
        },
      });
      Object.assign(window, { themeProbe: { handle, controller } });
    }, source);
  };
  const subscriptions = () =>
    page.evaluate(() =>
      (
        window as unknown as { themeSubscriptions(): { observers: number; media: number } }
      ).themeSubscriptions(),
    );
  await mountProbe(sample);
  await expect(page.locator('#theme-probe')).toHaveAttribute('data-state', 'ready');
  expect(await subscriptions()).toEqual({ observers: 1, media: 1 });
  await page.evaluate(() => {
    (
      window as unknown as { themeProbe: { handle: { release(): void } } }
    ).themeProbe.handle.release();
    document.documentElement.dataset.theme = 'dark';
  });
  expect(await subscriptions()).toEqual({ observers: 0, media: 0 });
  await expect(page.frameLocator('#theme-probe iframe').locator('html')).toHaveAttribute(
    'data-theme',
    'light',
  );
  await mountProbe(sample);
  await expect(page.frameLocator('#theme-probe iframe').locator('html')).toHaveAttribute(
    'data-theme',
    'dark',
  );
  await page.evaluate(() =>
    (
      window as unknown as { themeProbe: { controller: AbortController } }
    ).themeProbe.controller.abort(),
  );
  await expect(page.locator('#theme-probe iframe')).toHaveCount(0);
  expect(await subscriptions()).toEqual({ observers: 0, media: 0 });
  await mountProbe(sample);
  await expect(page.locator('#theme-probe')).toHaveAttribute('data-state', 'ready');
  await page.evaluate(() =>
    (
      window as unknown as { themeProbe: { handle: { destroy(): void } } }
    ).themeProbe.handle.destroy(),
  );
  expect(await subscriptions()).toEqual({ observers: 0, media: 0 });
  await mountProbe(
    '<button onclick="document.open();document.write(\'Replacement\');document.close()">Replace document</button>',
  );
  await page
    .frameLocator('#theme-probe iframe')
    .getByRole('button', { name: 'Replace document' })
    .click();
  await expect(page.locator('#theme-probe')).toHaveAttribute('data-state', 'navigation');
  expect(await subscriptions()).toEqual({ observers: 0, media: 0 });
  // No deadline fails a renderer that never answers: a long wait on the page's clock leaves it
  // loading, still subscribed, until leaving the page (the abort) removes it.
  await page.clock.install();
  await page.evaluate(() => delete document.getElementById('theme-probe')!.dataset.state);
  await page.route('**/renderer.html', (route) =>
    route.fulfill({ contentType: 'text/html', body: '<p>No bootstrap</p>' }),
  );
  await mountProbe(sample);
  await expect(page.locator('#theme-probe iframe')).toHaveCount(1);
  await page.clock.fastForward(60_000);
  await expect(page.locator('#theme-probe')).not.toHaveAttribute('data-state', /.+/);
  await expect(page.locator('#theme-probe iframe')).toHaveCount(1);
  expect(await subscriptions()).toEqual({ observers: 1, media: 1 });
  await page.evaluate(() =>
    (
      window as unknown as { themeProbe: { controller: AbortController } }
    ).themeProbe.controller.abort(),
  );
  await expect(page.locator('#theme-probe iframe')).toHaveCount(0);
  expect(await subscriptions()).toEqual({ observers: 0, media: 0 });
});

test('fixed-look CSS keeps its palette while Colab overwrites an author-pinned theme', async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: 'light' });
  const source = `<!doctype html><html data-theme="dark"><head><style>
    :root { color-scheme: light; --paper: #fafafa; --ink: #343434; }
    body { background: var(--paper); color: var(--ink); }
    </style></head><body><h1>Review notes</h1>
    <script>document.body.dataset.initialTheme = document.documentElement.dataset.theme;</script>
    </body></html>`;
  await mount(page, source);
  const child = page.frameLocator('#layout-fixture iframe');
  await expect(child.locator('body')).toHaveAttribute('data-initial-theme', 'light');
  for (const theme of ['dark', 'light'] as const) {
    await choose(page, theme);
    await expect(child.locator('html')).toHaveAttribute('data-theme', theme);
    await expect(child.locator('html')).toHaveCSS('color-scheme', 'light');
    await expect(child.locator('body')).toHaveCSS('background-color', 'rgb(250, 250, 250)');
    await expect(child.locator('body')).toHaveCSS('color', 'rgb(52, 52, 52)');
  }
});
