import { expect, test } from '@playwright/test';
import { mount, openChat, attach, send, actions } from '../test/attachment-page.js';
const captures = process.env.COLAB_2554_COMPOSER_CAPTURE_DIR;
for (const surface of ['chat', 'thread'] as const)
  for (const width of [1440, 390])
    for (const scheme of ['light', 'dark'] as const)
      test(`pending attachments share the input edge without editing or sending ${surface} ${width} ${scheme}`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.emulateMedia({ colorScheme: scheme });
        const bytes = Buffer.from(
          await page.evaluate(() => {
            const canvas = document.createElement('canvas');
            canvas.width = 600;
            canvas.height = 360;
            const c = canvas.getContext('2d')!;
            const gradient = c.createLinearGradient(0, 0, 600, 360);
            gradient.addColorStop(0, '#4387a7');
            gradient.addColorStop(1, '#a3be8c');
            c.fillStyle = gradient;
            c.fillRect(0, 0, 600, 360);
            return canvas.toDataURL('image/png').split(',')[1];
          }),
          'base64',
        );
        const raster = (name: string) => ({ name, mimeType: 'image/png', buffer: bytes });
        await mount(page);
        const { panel, input } = await (async () => {
          if (surface === 'chat') return openChat(page);
          const quote = page.frameLocator('#ask-page-fixture iframe').locator('#selected');
          await quote.evaluate((node) => {
            const selection = getSelection()!;
            const range = document.createRange();
            range.selectNodeContents(node);
            selection.removeAllRanges();
            selection.addRange(range);
          });
          await page.getByTestId('selection-ask').click();
          return {
            panel: page.locator('.comment-thread'),
            input: page.getByRole('combobox', { name: 'Message', exact: true }),
          };
        })();
        await input.fill('A posted image at the message edge.');
        await attach(page, [raster('posted.png')]);
        await send(page).click();
        await expect(panel.locator('.attachment-thumbnail img')).toBeVisible();
        await input.fill('Pending images follow this input text.');
        const editor = await input.elementHandle();
        const before = await actions(page);
        await attach(page, [
          raster('pending-a.png'),
          raster('pending-b.png'),
          {
            name: 'notes-with-a-long-filename.txt',
            mimeType: 'text/plain',
            buffer: Buffer.from('Notes'),
          },
        ]);
        const row = panel.locator('.attachment-pending-row');
        const chips = panel.getByTestId('attachment-chip');
        await expect(chips).toHaveCount(3);
        await expect(row.locator('img')).toHaveCount(2);
        await expect(row.locator('img').first()).toHaveAttribute('src', /^data:image\/png;base64,/);
        await expect(input).toHaveText('Pending images follow this input text.', {
          useInnerText: true,
        });
        expect(await input.evaluate((node, original) => node === original, editor)).toBe(true);
        expect(await actions(page)).toEqual(before);
        const geometry = await row.evaluate((row) => {
          const shell = row.closest('.rich-input-surface')!;
          const input = shell.querySelector('.rich-input-field')!;
          const body = input.querySelector('p')!.getBoundingClientRect();
          const attachment = row.firstElementChild!.getBoundingClientRect();
          const image = row.querySelector('img')!.getBoundingClientRect();
          const turn = document.querySelector('.conversation-turn')!;
          return {
            outsideText: !input.contains(row),
            left: Math.abs(body.left - attachment.left),
            imageWidth: image.width,
            imageHeight: image.height,
            postedLeft: Math.abs(
              turn.querySelector('.comment-body')!.getBoundingClientRect().left -
                turn.querySelector('.attachment-thumbnail')!.getBoundingClientRect().left,
            ),
            rightToSend: Math.abs(
              shell.getBoundingClientRect().right -
                row
                  .closest('.annotation-compose')!
                  .querySelector('[data-variant="primary"]')!
                  .getBoundingClientRect().right,
            ),
            inputToMessage: Math.abs(
              body.left - turn.querySelector('.comment-body')!.getBoundingClientRect().left,
            ),
            documentOverflow: document.documentElement.scrollWidth > innerWidth,
          };
        });
        expect(geometry.outsideText).toBe(true);
        expect(geometry.left).toBeLessThanOrEqual(1);
        expect(geometry.postedLeft).toBeLessThanOrEqual(1);
        expect(geometry.inputToMessage).toBeLessThanOrEqual(1);
        expect(geometry.rightToSend).toBeLessThanOrEqual(1);
        expect(geometry.imageWidth).toBe(32);
        expect(geometry.imageHeight).toBe(32);
        expect(geometry.documentOverflow).toBe(false);
        if (captures) {
          await page.mouse.move(0, 0);
          await page.screenshot({
            path: `${captures}/2554-pending-${surface}-alignment-${width}-${scheme}.png`,
          });
        }
        await chips
          .first()
          .getByRole('button', { name: 'Remove pending-a.png', exact: true })
          .press('Enter');
        await expect(chips).toHaveCount(2);
        await expect(input).toBeFocused();
        expect(await actions(page)).toEqual(before);
        expect(await input.evaluate((node, original) => node === original, editor)).toBe(true);
        await page.keyboard.press('End');
        await page.keyboard.type('!');
        await page.keyboard.press('ControlOrMeta+z');
        await expect(input).toHaveText('Pending images follow this input text.', {
          useInnerText: true,
        });
        await send(page).click();
        await expect(chips).toHaveCount(0);
        expect(await actions(page)).toContain('message:Pending images follow this input text.:2');
      });

test('many pending filenames stay in one horizontal row without page overflow', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 900 });
  await mount(page);
  const { panel, input } = await openChat(page);
  await input.fill('Many local files');
  await attach(
    page,
    Array.from({ length: 16 }, (_, index) => ({
      name: `long-local-filename-${index}.txt`,
      mimeType: 'text/plain',
      buffer: Buffer.from('Notes'),
    })),
  );
  const row = panel.locator('.attachment-pending-row');
  await expect(row.locator('li')).toHaveCount(16);
  expect(
    await row.evaluate((row) => ({
      overflow: row.scrollWidth > row.clientWidth,
      oneRow:
        new Set([...row.children].map((child) => child.getBoundingClientRect().top)).size === 1,
      pageFits: document.documentElement.scrollWidth <= innerWidth,
    })),
  ).toEqual({ overflow: true, oneRow: true, pageFits: true });
  expect(await actions(page)).toEqual([]);
  await expect(input).toHaveText('Many local files', { useInnerText: true });
});

for (const surface of ['chat', 'thread'] as const)
  for (const [width, scheme] of [
    [1440, 'light'],
    [390, 'dark'],
  ] as const)
    test(`panel drop keeps one local draft and shows refusals ${surface} ${width} ${scheme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: scheme });
      await mount(page);
      let panel, input;
      if (surface === 'chat') ({ panel, input } = await openChat(page));
      else {
        await page
          .frameLocator('#ask-page-fixture iframe')
          .locator('#selected')
          .evaluate((node) => {
            const range = document.createRange();
            range.selectNodeContents(node);
            const selection = getSelection()!;
            selection.removeAllRanges();
            selection.addRange(range);
          });
        await page.getByTestId('selection-ask').click();
        panel = page.locator('.comment-thread');
        input = page.getByRole('combobox', { name: 'Message', exact: true });
      }
      await input.fill('Text stays in this draft.');
      const editor = await input.elementHandle();
      const before = await actions(page);
      const drag = await page.evaluateHandle(() => {
        const data = new DataTransfer();
        data.items.add(new File(['local notes'], 'dropped.txt', { type: 'text/plain' }));
        return data;
      });
      const overlay = panel.getByTestId('attachment-drop-overlay');
      await panel.dispatchEvent('dragenter', { dataTransfer: drag });
      await expect(overlay).toHaveText(
        `⤓ Drop to attach to ${surface === 'chat' ? 'Chat' : 'this thread'}`,
      );
      if (captures)
        await page.screenshot({
          path: `${captures}/2554-overlay-ok-${surface}-${width}-${scheme}.png`,
        });
      // Enter the child before leaving the old child: a native nested crossing stays visible.
      await input.dispatchEvent('dragenter', { dataTransfer: drag });
      await panel
        .locator('.conversation-window-bar')
        .dispatchEvent('dragleave', { dataTransfer: drag });
      await expect(overlay).toBeVisible();
      await input.dispatchEvent('drop', { dataTransfer: drag });
      await expect(overlay).toHaveCount(0);
      await expect(panel.getByTestId('attachment-chip')).toHaveCount(1); // Bubbling must not add twice.
      expect(await actions(page)).toEqual(before);
      expect(await input.evaluate((node, original) => node === original, editor)).toBe(true);
      await expect(input).toHaveText('Text stays in this draft.', { useInnerText: true });
      const rejected = await page.evaluateHandle(() => {
        const data = new DataTransfer();
        data.items.add(new File([], 'empty.txt', { type: 'text/plain' }));
        return data;
      });
      await panel.dispatchEvent('dragenter', { dataTransfer: rejected });
      await expect(overlay).toHaveText('✗ Empty files cannot be attached.');
      if (captures)
        await page.screenshot({
          path: `${captures}/2554-overlay-refused-${surface}-${width}-${scheme}.png`,
        });
      await panel.dispatchEvent('drop', { dataTransfer: rejected });
      await expect(overlay).toHaveCount(0);
      await expect(panel.getByTestId('attachment-chip')).toHaveCount(1);
      expect(await actions(page)).toEqual(before);
      await panel.dispatchEvent('dragenter', { dataTransfer: drag });
      await panel.dispatchEvent('dragleave', { dataTransfer: drag });
      await expect(overlay).toHaveCount(0);
      await panel.dispatchEvent('dragenter', { dataTransfer: drag });
      await input.press('Escape');
      await expect(overlay).toHaveCount(0);
      await expect(input).toBeFocused();
      // A pending draft survives the publish fence; blocked panels cannot add or send.
      await page.evaluate(
        async (path) => (await import(path)).block(),
        '/test/ask-page-browser.tsx',
      );
      await panel.dispatchEvent('dragenter', { dataTransfer: drag });
      await expect(overlay).toHaveText('✗ Attachments are unavailable here.');
      await panel.dispatchEvent('drop', { dataTransfer: drag });
      await expect(panel.getByTestId('attachment-chip')).toHaveCount(1);
      expect(await actions(page)).toEqual(before);
      await drag.dispose();
      await rejected.dispose();
    });
