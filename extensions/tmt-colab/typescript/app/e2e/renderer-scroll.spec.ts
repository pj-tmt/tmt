import { headerHeight } from './tokens.js';
import { expect, test, type Page } from '@playwright/test';

/** What trusted content takes between the header and the page frame, margins included: the reader's
 * status and note, or the owner page's status band. */
const noteSize = (page: Page) =>
  page
    .locator('.reader-note, .reader-status-row, .work-band')
    .evaluateAll((nodes) =>
      nodes
        .filter((node) => node.getClientRects().length)
        .reduce(
          (sum, node) =>
            sum +
            node.getBoundingClientRect().height +
            Number.parseFloat(getComputedStyle(node).marginTop) +
            Number.parseFloat(getComputedStyle(node).marginBottom),
          0,
        ),
    );

async function mount(page: Page, source: string, reader: boolean, readerOnly = false) {
  await page.goto('/');
  // The shipped reader bundle never loads the owner stylesheet; drop it so the reader rules stand alone.
  if (readerOnly)
    await page.evaluate(() =>
      document
        .querySelectorAll('style[data-vite-dev-id$="/src/style.css"]')
        .forEach((n) => n.remove()),
    );
  await page.evaluate(() => {
    const firstHeight = new Promise<void>((resolve) => {
      const receive = (event: MessageEvent) => {
        const frame = document.querySelector('iframe');
        if (
          frame &&
          event.source === frame.contentWindow &&
          event.data?.type === 'colab.render.height' &&
          event.data.renderId === frame.dataset.renderId
        ) {
          window.removeEventListener('message', receive);
          resolve();
        }
      };
      window.addEventListener('message', receive);
    });
    Object.assign(window, { firstHeight });
  });
  await page.evaluate(
    async ({ source, reader }) => {
      const path = '/test/page-layout-browser.tsx';
      const fixture = await import(path);
      await fixture[reader ? 'mountReader' : 'mount'](source);
    },
    { source, reader },
  );
  // The owner page's status sits in the shared bar; the reader's content row carries it.
  await expect(
    page.locator(reader ? '.reader-status-row .status' : '.tmt-ui-topbar:visible .status'),
  ).toContainText('Live');
  await page.evaluate(() => (window as unknown as { firstHeight: Promise<void> }).firstHeight);
}
const html = `<style>body{margin:0}main{padding:24px}.space{height:2200px}.tail{height:1000px}</style>
<main><a href="#target">Jump to target</a><button onclick="document.getElementById('grow').style.height='600px'">Grow content</button>
<div class="space"></div><div id="grow"></div><h2 id="target">Target quote</h2><div class="tail"></div></main>`;

for (const reader of [false, true]) {
  test(`${reader ? 'reader' : 'owner'}: content drives one window scroll, resizing and fragment anchors`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await mount(page, html, reader);
    const frame = page.locator('iframe'),
      child = page.frameLocator('iframe');
    await expect.poll(async () => (await frame.boundingBox())?.height ?? 0).toBeGreaterThan(3200);
    await expect(frame).toHaveAttribute('scrolling', 'no');
    await expect(frame).toHaveAttribute('data-scroll-mode', 'window');
    await expect
      .poll(() =>
        child.locator('html').evaluate((node) => node.scrollHeight <= node.clientHeight + 1),
      )
      .toBe(true);
    const initial = (await frame.boundingBox())!.height;
    await child.getByRole('button', { name: 'Grow content' }).click();
    await expect
      .poll(async () => (await frame.boundingBox())?.height ?? 0)
      .toBeGreaterThan(initial + 590);
    await child.getByRole('link', { name: 'Jump to target' }).click();
    await expect.poll(() => page.evaluate(() => window.scrollY)).toBeGreaterThan(2000);
    expect(
      await child.locator('html').evaluate((node) => node.ownerDocument.defaultView!.scrollY),
    ).toBe(0);
    const targetTop = await child
      .locator('#target')
      .evaluate((node) => node.getBoundingClientRect().top);
    // The status band stays pinned under the bar, so the anchor lands below both.
    const pinned = (await page.locator('.work-band:visible').boundingBox())?.height ?? 0;
    expect(
      Math.abs((await frame.boundingBox())!.y + targetTop - headerHeight(1440) - pinned),
    ).toBeLessThan(2);
    await child.locator('#grow').evaluate((node: HTMLElement) => {
      node.style.height = '0px';
    });
    await expect
      .poll(async () => (await frame.boundingBox())?.height ?? 0)
      .toBeLessThan(initial + 2);
    await page.setViewportSize({ width: 390, height: 844 });
    await expect(frame).toHaveAttribute('data-scroll-mode', 'window');
    await expect
      .poll(() =>
        child.locator('html').evaluate((node) => node.scrollHeight <= node.clientHeight + 1),
      )
      .toBe(true);
    expect((await frame.boundingBox())!.width).toBe(
      await page.evaluate(() => document.documentElement.clientWidth),
    );
  });

  test(`${reader ? 'reader' : 'owner'}: viewport-coupled growth falls back to a bounded inner scroll`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await mount(
      page,
      '<style>body{margin:0}.grow{height:calc(100vh + 120px)}</style><div class="grow">Viewport-coupled page</div>',
      reader,
    );
    const frame = page.locator('iframe');
    await expect(frame).toHaveAttribute('data-scroll-mode', 'frame');
    await expect(frame).toHaveAttribute('scrolling', 'auto');
    expect((await frame.boundingBox())!.height).toBe(
      900 - headerHeight(1440) - (await noteSize(page)),
    );
    await page
      .frameLocator('iframe')
      .locator('html')
      .evaluate((node) => node.ownerDocument.defaultView!.scrollTo(0, 50));
    expect(
      await page
        .frameLocator('iframe')
        .locator('html')
        .evaluate((node) => node.ownerDocument.defaultView!.scrollY),
    ).toBeGreaterThan(0);
    await page.setViewportSize({ width: 390, height: 700 });
    await expect(
      page.locator(reader ? '.tmt-ui-header:visible' : '.tmt-ui-topbar:visible'),
    ).toHaveCSS('height', `${headerHeight(390)}px`);
    // The frame fills the viewport below the header (and, in the reader, its note). In the TMT shell the header is the shell's bar
    // (plus the module menu) and the frame fills the shell's main area; the reader has no shell,
    // so its fixed header's token height sits above the frame.
    await expect
      .poll(async () => {
        const box = (await frame.boundingBox())!;
        const shellMain = page.locator('.tmt-ui-shell-main');
        const main = (await shellMain.count()) ? await shellMain.boundingBox() : null;
        return [
          box.y + box.height,
          box.y - (main?.y ?? headerHeight(390)) - (await noteSize(page)),
        ];
      })
      .toEqual([700, 0]);
    await expect(frame).toHaveAttribute('data-scroll-mode', 'frame');
  });

  test(`${reader ? 'reader' : 'owner'}: a page shorter than the viewport does not scroll the document`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await mount(page, '<style>body{margin:0}</style><main>A short page</main>', reader);
    const frame = page.locator('iframe');
    await expect(frame).toHaveAttribute('data-scroll-mode', 'window');
    // The reader's note above the frame takes room from the frame's minimum height, so the
    // document is exactly one viewport tall, with and without the note.
    await expect
      .poll(() =>
        page.evaluate(
          () => document.documentElement.scrollHeight - document.documentElement.clientHeight,
        ),
      )
      .toBeLessThanOrEqual(0);
    if (reader) expect(await noteSize(page)).toBeGreaterThan(0);
  });
  test(`${reader ? 'reader' : 'owner'}: expanding the status panel reallocates short-page viewport room`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await mount(page, '<style>body{margin:0}</style><main>A short page</main>', reader);
    const frame = page.locator('iframe');
    const before = (await frame.boundingBox())!.height;
    const panel = page.getByRole('region', { name: 'Work on this page' });
    await panel.getByRole('button', { name: 'Expand work panel' }).click();
    await expect.poll(async () => (await frame.boundingBox())!.height).toBeLessThan(before);
    await expect
      .poll(() =>
        page.evaluate(
          () => document.documentElement.scrollHeight - document.documentElement.clientHeight,
        ),
      )
      .toBeLessThanOrEqual(0);
    await panel.getByRole('button', { name: 'Collapse work panel' }).click();
    await expect.poll(async () => (await frame.boundingBox())!.height).toBe(before);
  });
}

test('identical normalized slot requests cannot mask authored viewport feedback', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/src/renderer.ts';
    const { mountRenderer } = await import(path);
    const host = document.createElement('div');
    document.getElementById('root')!.replaceChildren(host);
    const controller = new AbortController();
    const renderer = await mountRenderer(
      host,
      '<style>body{margin:0}</style><div style="height:calc(100vh + 120px)">Authored feedback</div>',
      { signal: controller.signal, onState() {} },
    );
    let reports = 0;
    window.addEventListener(
      'message',
      (event) => {
        if (
          event.source !== host.querySelector('iframe')?.contentWindow ||
          event.data?.type !== 'colab.render.height' ||
          event.data.renderId !== renderer.snapshot.renderId
        )
          return;
        // Different raw values normalize to the same trusted footprint each time.
        renderer.slots([
          { id: '10000000-0000-4000-8000-000000000001', height: ++reports % 2 ? 20.2 : 20.7 },
        ]);
      },
      { signal: controller.signal },
    );
  });
  await expect(page.locator('iframe')).toHaveAttribute('data-scroll-mode', 'frame');
  await expect(page.locator('iframe')).toHaveAttribute('scrolling', 'auto');
  expect((await page.locator('iframe').boundingBox())!.height).toBe(900);
});

test('layout claims are bound to the current frame/render and bounded even when author-forged', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mount(page, '<p>Bound cosmetic claims</p>', false);
  const frame = page.locator('iframe'),
    child = page.frameLocator('iframe');
  const renderId = await frame.getAttribute('data-render-id');
  const before = (await frame.boundingBox())!.height;
  await page.evaluate((renderId) => {
    window.postMessage({ type: 'colab.render.height', renderId, height: 999999 }, '*');
  }, renderId);
  await child.locator('html').evaluate((node, renderId) => {
    const view = node.ownerDocument.defaultView!;
    for (const height of [-1, NaN, Infinity])
      view.parent.postMessage({ type: 'colab.render.height', renderId, height }, '*');
    view.parent.postMessage(
      { type: 'colab.render.height', renderId: 'stale', height: 999999 },
      '*',
    );
    view.parent.postMessage(
      { type: 'colab.render.height', renderId, height: 999999, extra: true },
      '*',
    );
  }, renderId);
  // Flush postMessage tasks through a later message from the same frame.
  await child.locator('html').evaluate((node, renderId) => {
    node.ownerDocument.defaultView!.parent.postMessage(
      { type: 'colab.render.anchor', renderId, top: 0 },
      '*',
    );
  }, renderId);
  await expect(frame).toHaveAttribute('data-scroll-mode', 'window');
  expect((await frame.boundingBox())!.height).toBe(before);
  await child.locator('html').evaluate((node, renderId) => {
    node.ownerDocument.defaultView!.parent.postMessage(
      { type: 'colab.render.height', renderId, height: 1e12 },
      '*',
    );
  }, renderId);
  await expect.poll(async () => (await frame.boundingBox())?.height).toBe(1_000_000);
  await expect(frame).toHaveAttribute('data-scroll-mode', 'window');
  // A burst is coalesced to bounded frame mutations, retaining its latest claim.
  await page.evaluate(() => {
    const node = document.querySelector('iframe')!;
    Object.assign(window, { layoutMutations: 0 });
    const observer = new MutationObserver((records) => {
      const probe = window as unknown as { layoutMutations: number };
      probe.layoutMutations += records.length;
    });
    observer.observe(node, { attributes: true, attributeFilter: ['style'] });
    Object.assign(window, { layoutObserver: observer });
  });
  await child.locator('html').evaluate((node, renderId) => {
    for (let height = 1100; height < 1150; height++)
      node.ownerDocument.defaultView!.parent.postMessage(
        { type: 'colab.render.height', renderId, height },
        '*',
      );
  }, renderId);
  await expect.poll(async () => (await frame.boundingBox())?.height).toBe(1149);
  const mutations = await page.evaluate(() => {
    const probe = window as unknown as {
      layoutMutations: number;
      layoutObserver: MutationObserver;
    };
    probe.layoutObserver.disconnect();
    return probe.layoutMutations;
  });
  expect(mutations).toBeLessThanOrEqual(3);
});

for (const reader of [false, true]) {
  for (const colorScheme of ['light', 'dark'] as const) {
    test(`${reader ? 'reader' : 'owner'}: a page without its own background is one white canvas in ${colorScheme}`, async ({
      page,
    }) => {
      await page.emulateMedia({ colorScheme });
      await mount(page, '<p>Unstyled</p>', reader, reader);
      const frame = page.locator('iframe');
      await expect(frame).toHaveCSS('background-color', 'rgb(255, 255, 255)');
      await expect(frame).toHaveCSS('color-scheme', 'light');
      await expect(page.locator('.frame-host')).toHaveCSS('background-color', 'rgb(255, 255, 255)');
    });
  }
}
