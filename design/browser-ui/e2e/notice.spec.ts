import { expect, test } from '@playwright/test';
import { controlBoundaries, focusFrames, textAtSurfaceEdge } from './audits';

for (const theme of ['light', 'dark'] as const)
  for (const width of [1586, 390]) {
    test(`Notice variants and command composition at ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 992 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.goto('/?frame=notice&theme=' + theme);
      const pair = (title: string) =>
        page
          .locator('.gallery-pair')
          .filter({
            has: page.getByRole('heading', { name: title, exact: true }),
          })
          .locator('.gallery-column[data-theme="' + theme + '"]');
      const neutral = pair('BrowserNotice tones');
      const tinted = pair('BrowserNotice tinted tones');
      for (const tone of ['waiting', 'working', 'review', 'blocked', 'muted']) {
        const plain = neutral.locator('.tmt-ui-notice[data-tone="' + tone + '"]');
        const soft = tinted.locator('.tmt-ui-notice[data-tone="' + tone + '"]');
        await expect(plain).not.toHaveAttribute('data-tinted');
        await expect(soft).toHaveAttribute('data-tinted', '');
        const measured = await soft.evaluate((node, state) => {
          const style = getComputedStyle(node);
          const rgb = (hex: string) =>
            'rgb(' +
            (hex.match(/[a-f0-9]{2}/gi) ?? []).map((v) => parseInt(v, 16)).join(', ') +
            ')';
          const expected = (token: string) => rgb(style.getPropertyValue(token).trim());
          const luminance = (colour: string) => {
            const channels = (colour.match(/[\d.]+/g) ?? [])
              .slice(0, 3)
              .map(Number)
              .map((value) => {
                const v = value / 255;
                return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
              });
            return channels[0]! * 0.2126 + channels[1]! * 0.7152 + channels[2]! * 0.0722;
          };
          const fill = luminance(style.backgroundColor);
          const contrast = (colour: string) => {
            const ink = luminance(colour);
            return (Math.max(ink, fill) + 0.05) / (Math.min(ink, fill) + 0.05);
          };
          return {
            actual: style.backgroundColor,
            expected: expected(
              '--tmt-ui-surface-' + (state === 'muted' ? 'sheet' : state + '-soft'),
            ),
            neutral: expected('--tmt-ui-surface-sheet'),
            rule: expected('--tmt-ui-surface-rule'),
            text: contrast(style.color),
            mark: contrast(getComputedStyle(node.querySelector('.tmt-ui-notice-mark')!).color),
            state: contrast(
              getComputedStyle(node.querySelector('.tmt-ui-notice-mark > :not([aria-hidden])')!)
                .color,
            ),
            caption: contrast(
              getComputedStyle(node.querySelector('.tmt-ui-notice-eyebrow')!).color,
            ),
          };
        }, tone);
        expect(measured.actual).toBe(measured.expected);
        await expect(plain).toHaveCSS('background-color', measured.neutral);
        expect(measured.text).toBeGreaterThanOrEqual(4.5);
        expect(measured.mark).toBeGreaterThanOrEqual(3);
        expect(measured.state).toBeGreaterThanOrEqual(4.5);
        expect(measured.caption).toBeGreaterThanOrEqual(4.5);
        for (const side of ['top', 'right', 'bottom', 'left']) {
          await expect(soft).toHaveCSS('border-' + side + '-width', '1px');
          await expect(soft).toHaveCSS('border-' + side + '-color', measured.rule);
        }
        await expect(soft).toHaveCSS('border-top-left-radius', '8px');
        expect((await soft.boundingBox())!.height).toBe((await plain.boundingBox())!.height);
      }
      const example = pair('BrowserNotice tinted with command');
      const command = example.locator('.tmt-ui-command');
      await expect(command.locator('code')).toHaveText('tmt colab settings');
      await expect(example.getByRole('status')).toHaveCount(0);
      const copy = example.getByRole('button', { name: 'Copy command', exact: true });
      await copy.focus();
      await page.keyboard.press('Enter');
      await expect(example.getByRole('status')).toHaveText('Copied');
      await expect(copy).toBeFocused();
      await expect(command.locator('code')).toHaveText('tmt colab settings');
      for (const qualifier of [
        ".gallery-pair:has(.tmt-ui-notice[data-tone='review']:not([data-tinted]))",
        ".gallery-pair:has(.tmt-ui-notice[data-tone='review'][data-tinted])",
        '.gallery-pair:has(.tmt-ui-command)',
      ]) {
        const scope = '#notice ' + qualifier + ' .gallery-column[data-theme="' + theme + '"]';
        expect((await controlBoundaries(page, scope)).borders.misses).toEqual([]);
        expect(await textAtSurfaceEdge(page, scope)).toEqual([]);
        const frames = await focusFrames(page, scope);
        expect(frames.checked).toBeGreaterThan(0);
        expect(frames.clipped).toEqual([]);
        expect(frames.doubled).toEqual([]);
      }
    });
  }
