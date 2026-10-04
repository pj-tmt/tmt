import { expect, test, type Page } from '@playwright/test';

async function mount(page: Page, source: string, reader: boolean) {
  await page.goto('/');
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
  await expect(page.locator(reader ? '.reader-bar .status' : '.page-bar .status')).toContainText(
    reader ? 'Live' : 'Live preview',
  );
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
    expect(Math.abs((await frame.boundingBox())!.y + targetTop - 56)).toBeLessThan(2);
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
    expect((await frame.boundingBox())!.height).toBe(844);
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
    await expect.poll(async () => (await frame.boundingBox())?.height).toBe(644);
    await expect(frame).toHaveAttribute('data-scroll-mode', 'frame');
  });
}

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
