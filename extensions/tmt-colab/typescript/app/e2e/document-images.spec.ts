import { expect, test, type Page } from '@playwright/test';
import type { mountRenderer } from '../src/renderer.js';

const PNG =
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==';
type Probe = {
  handle: Awaited<ReturnType<typeof mountRenderer>>;
  reset(): void;
  finish(): void;
  reads: string[];
};
async function mount(page: Page, source: string, backend = true, budget = false) {
  await page.goto('/');
  await page.evaluate(
    async ({ source, backend, budget, png }) => {
      const path = '/src/renderer.ts';
      const { mountRenderer } = await import(path);
      const host = document.createElement('div');
      host.id = 'image-probe';
      document.body.replaceChildren(host);
      let scope = 'admitted',
        finish = () => {};
      const reads: string[] = [];
      const descriptors = budget
        ? Array.from({ length: 9 }, (_, i) => ({
            attachmentId: String(i + 1).repeat(8) + '-1111-4111-8111-111111111111',
            mediaType: 'image/png',
            plaintextBytes: String(8 * 1024 * 1024),
          }))
        : [
            {
              attachmentId: '11111111-1111-4111-8111-111111111111',
              mediaType: 'image/png',
              plaintextBytes: '68',
            },
            {
              attachmentId: '22222222-2222-4222-8222-222222222222',
              mediaType: 'application/pdf',
              plaintextBytes: '68',
            },
            {
              attachmentId: '33333333-3333-4333-8333-333333333333',
              mediaType: 'image/png',
              plaintextBytes: '68',
            },
          ];
      const binding = {
        scope: () => scope,
        open: async (_reference: unknown, descriptor: { attachmentId: string }) => {
          reads.push(descriptor.attachmentId);
          if (!budget && descriptor.attachmentId.startsWith('33333333'))
            await new Promise<void>((resolve) => {
              finish = resolve;
            });
          if (scope !== 'admitted') throw new Error('admission ended');
          return Uint8Array.from(atob(png), (c) => c.charCodeAt(0));
        },
      };
      const input = { descriptors, binding: backend ? binding : undefined };
      const controller = new AbortController();
      const handle = await mountRenderer(host, source, {
        signal: controller.signal,
        images: input,
        onState: (state: string) => {
          host.dataset.state = state;
        },
      });
      // This fixture owns the host width; exercise authored wide columns as well.
      host.querySelector('iframe')!.style.width = '100%';
      Object.assign(window, {
        imageProbe: {
          handle,
          reads,
          finish: () => finish(),
          reset: () => {
            scope = 'revoked';
            handle.images(input);
            finish();
          },
        },
      });
    },
    { source, backend, budget, png: PNG },
  );
  await expect(page.locator('#image-probe')).toHaveAttribute('data-state', 'ready');
}

for (const width of [1440, 390])
  for (const dark of [false, true]) {
    test(`document images: authored ${dark ? 'dark' : 'light'}, ${width}, admitted reads and inert placeholders`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await mount(
        page,
        `<style>body{font:16px system-ui;background:${dark ? '#18222b' : '#fff'};color:${dark ? '#eee' : '#17212b'};padding:16px}img{display:block;margin:12px 0}p img{display:inline;margin:0}</style>
      <h1>Document images</h1>
      <a href="#example"><img id="resolved" src="colab-attachment:11111111" alt="Verified image" width="240" height="120"></a>
      <img id="unknown" src="colab-attachment:99999999" alt="An unavailable image with a long description that wraps safely across narrow authored pages" width="240" height="100">
      <img src="colab-attachment:22222222" alt="A PDF is not an image" width="240" height="90">
      <p>Unsized inline: <img src="colab-attachment:99999998" alt="Inline image"></p>
      <img src="colab-attachment:99999997" alt="">
      <img src="colab-attachment:99999996" alt="Small icon" width="80" height="40">
      <img src="colab-attachment:33333333" alt="Loading image" width="240" height="90">`,
      );
      const frame = page.frameLocator('#image-probe iframe');
      await expect(frame.locator('#resolved')).toHaveAttribute('src', /^data:image\/png;base64,/);
      await expect
        .poll(() =>
          frame.locator('#resolved').evaluate((image: HTMLImageElement) => image.naturalWidth),
        )
        .toBe(1);
      await expect(frame.getByRole('img', { name: 'A PDF is not an image' })).toContainText(
        'Not shown: unsupported image type',
      );
      await expect(frame.getByRole('img', { name: 'Loading image', exact: true })).toHaveText(
        'Loading image',
      );
      await expect(
        frame.getByRole('img', { name: 'Small icon. Image unavailable', exact: true }),
      ).toHaveText('');
      await expect(
        frame.getByRole('img', {
          name: 'A PDF is not an image. Not shown: unsupported image type',
          exact: true,
        }),
      ).toBeVisible();
      const unknown = frame.getByRole('img', { name: /^An unavailable image/ });
      expect(await unknown.evaluate((node) => getComputedStyle(node).overflow)).toBe('hidden');
      expect(
        await unknown
          .locator('span')
          .first()
          .evaluate((node) => getComputedStyle(node).webkitLineClamp),
      ).toBe('3');
      await expect(frame.locator('[data-colab-image][hidden]')).toHaveCount(1);
      await expect(frame.locator('[data-colab-image][hidden]')).toBeHidden();
      expect(
        await page.evaluate(() => (window as unknown as { imageProbe: Probe }).imageProbe.reads),
      ).toEqual(['11111111-1111-4111-8111-111111111111', '33333333-3333-4333-8333-333333333333']);
      await page.screenshot({
        path: `/private/tmp/colab-2-2556-evidence/images-${width}-${dark ? 'dark' : 'light'}.png`,
        fullPage: true,
      });
      await page.evaluate(() => (window as unknown as { imageProbe: Probe }).imageProbe.reset());
      await expect(frame.locator('img[src^="data:"]')).toHaveCount(0);
      await expect(frame.getByRole('img', { name: 'Verified image' })).toContainText(
        'Image unavailable',
      );
      await expect(frame.getByRole('img', { name: 'Loading image' })).toContainText(
        'Image unavailable',
      );
      await mount(
        page,
        `<style>body{font:16px system-ui;background:${dark ? '#18222b' : '#fff'};color:${dark ? '#eee' : '#17212b'};padding:16px}</style><h1>Images on this backend</h1><img src="colab-attachment:11111111" alt="A document image" width="240" height="100">`,
        false,
      );
      await expect(
        page.frameLocator('#image-probe iframe').getByRole('img', { name: 'A document image' }),
      ).toContainText('Image not available here');
      await page.screenshot({
        path: `/private/tmp/colab-2-2556-evidence/not-here-${width}-${dark ? 'dark' : 'light'}.png`,
        fullPage: true,
      });
      await mount(
        page,
        `<style>body{font:16px system-ui;background:${dark ? '#18222b' : '#fff'};color:${dark ? '#eee' : '#17212b'};padding:16px}img{width:100px;height:25px;display:block}</style><h1>Image limit</h1>${Array.from({ length: 9 }, (_, i) => `<img src="colab-attachment:${String(i + 1).repeat(8)}" alt="Image ${i + 1}">`).join('')}`,
        true,
        true,
      );
      await expect(
        page
          .frameLocator('#image-probe iframe')
          .getByRole('img', { name: 'Image 9. Not shown: image limit reached', exact: true }),
      ).toContainText('Not shown: image limit reached');
      expect(
        await page.evaluate(() => (window as unknown as { imageProbe: Probe }).imageProbe.reads),
      ).toHaveLength(8);
      await page.screenshot({
        path: `/private/tmp/colab-2-2556-evidence/limit-${width}-${dark ? 'dark' : 'light'}.png`,
        fullPage: true,
      });
    });
  }
test('a backend without document file access makes no reads and renders not-here placeholders', async ({
  page,
}) => {
  await mount(page, '<img src="colab-attachment:11111111" alt="Local file">', false);
  await expect(
    page.frameLocator('#image-probe iframe').getByRole('img', { name: 'Local file' }),
  ).toContainText('Image not available here');
  expect(
    await page.evaluate(() => (window as unknown as { imageProbe: Probe }).imageProbe.reads),
  ).toEqual([]);
});

test('a delayed document read stays alt-only until a definite host response', async ({ page }) => {
  await mount(
    page,
    '<img src="colab-attachment:33333333" alt="Pending image" width="240" height="90">',
  );
  const frame = page.frameLocator('#image-probe iframe');
  const pending = frame.getByRole('img', { name: 'Pending image', exact: true });
  await expect(pending).toHaveText('Pending image');
  await expect(pending.locator('span')).toHaveCount(1);
  await page.evaluate(() => (window as unknown as { imageProbe: Probe }).imageProbe.finish());
  await expect(frame.locator('img')).toHaveAttribute('src', /^data:image\/png;base64,/);
  await expect(frame.locator('[data-colab-image]')).toHaveCount(0);
});
