import { expect, test } from '@playwright/test';
import { mount, openChat, attach, send } from '../test/attachment-page.js';
const captures = process.env.COLAB_2554_CAPTURE_DIR;
for (const width of [1440, 390])
  for (const scheme of ['light', 'dark'] as const)
    test(`real page message attachment alignment ${width} ${scheme}`, async ({ page }) => {
      test.skip(!captures, 'Set COLAB_2554_CAPTURE_DIR to save real-page review captures.');
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: scheme });
      const raster = Buffer.from(
        await page.evaluate(() => {
          const canvas = document.createElement('canvas');
          canvas.width = 600;
          canvas.height = 360;
          const ctx = canvas.getContext('2d')!;
          const gradient = ctx.createLinearGradient(0, 0, 600, 360);
          gradient.addColorStop(0, '#4387a7');
          gradient.addColorStop(1, '#a3be8c');
          ctx.fillStyle = gradient;
          ctx.fillRect(0, 0, 600, 360);
          ctx.fillStyle = '#ffffff';
          ctx.font = '40px sans-serif';
          ctx.fillText('Image attachment', 32, 80);
          ctx.fillRect(32, 112, 180, 4);
          return canvas.toDataURL('image/png').split(',')[1];
        }),
        'base64',
      );
      for (const count of [1, 2, 6, 7]) {
        await mount(page);
        const { panel, input } = await openChat(page);
        await input.fill('Images and files stay aligned with this message.');
        await attach(page, [
          ...Array.from({ length: count }, (_, index) => ({
            name: `image-${index + 1}.png`,
            mimeType: 'image/png',
            buffer: raster,
          })),
          {
            name: 'review-notes-with-a-long-filename.txt',
            mimeType: 'text/plain',
            buffer: Buffer.from('Review notes'),
          },
        ]);
        await send(page).click();
        await expect(panel.locator('.attachment-thumbnail img')).toHaveCount(count > 6 ? 5 : count);
        const geometry = await panel.locator('.message-attachments').evaluate((node) => {
          const body = node
            .closest('.conversation-body')!
            .querySelector('.comment-body')!
            .getBoundingClientRect();
          const thumbnail = node.querySelector('.attachment-thumbnail')!.getBoundingClientRect();
          const card = node.querySelector('.message-file')!.getBoundingClientRect();
          return {
            thumbnail: Math.abs(thumbnail.left - body.left),
            card: Math.abs(card.left - body.left),
            width: thumbnail.width,
            height: thumbnail.height,
          };
        });
        expect(geometry.thumbnail).toBeLessThanOrEqual(1);
        expect(geometry.card).toBeLessThanOrEqual(1);
        if (count === 1) {
          expect(geometry.width).toBeLessThanOrEqual(200);
          expect(geometry.height).toBeLessThanOrEqual(120);
        } else expect(Math.abs(geometry.width - geometry.height)).toBeLessThanOrEqual(1);
        await page.screenshot({
          path: `${captures}/2554-chat-${count}-mixed-alignment-${width}-${scheme}.png`,
        });
        if (count === 7) {
          await panel.getByRole('button', { name: 'Show 2 more images' }).click();
          await expect(page.locator('.attachment-viewer img')).toBeVisible();
          await page.screenshot({ path: `${captures}/2554-chat-viewer-${width}-${scheme}.png` });
          await page.keyboard.press('Escape');
          await expect(panel.getByRole('button', { name: 'Show 2 more images' })).toBeFocused();
        }
      }
      await mount(page);
      const quote = page.frameLocator('#ask-page-fixture iframe').locator('#selected');
      await quote.evaluate((node) => {
        const selection = getSelection()!;
        const range = document.createRange();
        range.selectNodeContents(node);
        selection.removeAllRanges();
        selection.addRange(range);
      });
      await page.getByTestId('selection-ask').click();
      await page
        .getByRole('combobox', { name: 'Message', exact: true })
        .fill('The thread grid starts at the message edge.');
      await attach(
        page,
        Array.from({ length: 7 }, (_, index) => ({
          name: `image-${index + 1}.png`,
          mimeType: 'image/png',
          buffer: raster,
        })),
      );
      await send(page).click();
      await expect(page.locator('.attachment-thumbnail img')).toHaveCount(5);
      const alignment = await page
        .locator('.attachment-images')
        .evaluate((node) =>
          Math.abs(
            node.getBoundingClientRect().left -
              node
                .closest('.conversation-body')!
                .querySelector('.comment-body')!
                .getBoundingClientRect().left,
          ),
        );
      expect(alignment).toBeLessThanOrEqual(1);
      await page.screenshot({
        path: `${captures}/2554-thread-grid-alignment-${width}-${scheme}.png`,
      });
    });
