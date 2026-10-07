import { expect, test } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

const fixture = '/test/message-composer-browser.html';
for (const width of [1440, 390]) {
  for (const theme of ['light', 'dark'] as const) {
    test(`mention choices use available viewport outside the popover: ${width}px ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 844 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto(fixture);
      const input = page.getByRole('combobox', { name: 'Message', exact: true });
      await expect(input).toHaveText('', { useInnerText: true });
      const evidence = [];
      for (const position of ['Top', 'Bottom']) {
        await page.getByRole('button', { name: position, exact: true }).click();
        // Use trusted editing keys: fill('') selects the DOM before Lexical admits that selection.
        await input.press('ControlOrMeta+A');
        await input.press('Backspace');
        await expect(input).toHaveText('', { useInnerText: true });
        await input.pressSequentially('@');
        const list = page.getByRole('listbox');
        await expect(list).toBeVisible();
        await expect(list.getByRole('option')).toHaveCount(3);
        // A visible full list must also follow the current field, not retain an old position.
        await expect
          .poll(() =>
            list.evaluate((node) => {
              const menu = node.getBoundingClientRect();
              const field = document.querySelector('[contenteditable]')!.getBoundingClientRect();
              return Math.min(
                Math.abs(menu.top - field.bottom - 2),
                Math.abs(field.top - menu.bottom - 2),
              );
            }),
          )
          .toBeLessThanOrEqual(4);
        const geometry = await list.evaluate((node) => {
          const rect = (element: Element) => {
            const r = element.getBoundingClientRect();
            return { x: r.x, y: r.y, width: r.width, height: r.height, bottom: r.bottom };
          };
          const element = node as HTMLElement;
          const popover = document.querySelector('.annotation-popover')!;
          return {
            viewport: { width: innerWidth, height: innerHeight },
            list: rect(node),
            input: rect(document.querySelector('[contenteditable]')!),
            popover: rect(popover),
            portalParent: node.parentElement!.tagName,
            scrollHeight: element.scrollHeight,
            clientHeight: element.clientHeight,
            maxHeight: getComputedStyle(element).maxHeight,
            popoverOverflow: getComputedStyle(popover).overflowY,
            hitTargets: [...node.querySelectorAll('[role=option]')].every((option) => {
              const box = option.getBoundingClientRect();
              return option.contains(
                document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2),
              );
            }),
          };
        });
        evidence.push({ position, ...geometry });
        const directory = process.env.COLAB_COMPOSER_CAPTURE_DIR;
        if (directory) {
          mkdirSync(directory, { recursive: true });
          await page.screenshot({ path: `${directory}/${width}-${theme}-${position}.png` });
        }
        await input.press('Escape');
      }
      const directory = process.env.COLAB_COMPOSER_CAPTURE_DIR;
      if (directory)
        writeFileSync(`${directory}/${width}-${theme}-geometry.json`, JSON.stringify(evidence));
      // Three short options fit within the viewport at both positions; the popover is not their clip.
      for (const geometry of evidence) {
        expect(geometry.clientHeight).toBeGreaterThanOrEqual(geometry.scrollHeight);
        expect(geometry.hitTargets).toBe(true);
        expect(geometry.list.y).toBeGreaterThanOrEqual(8);
        expect(geometry.list.bottom).toBeLessThanOrEqual(geometry.viewport.height - 8);
      }
    });
  }
}

test('recipient selection keeps multiline surrounding text and performs no effects', async ({
  page,
}) => {
  await page.goto(fixture);
  const input = page.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toHaveText('', { useInnerText: true });
  await input.fill('Before @O\nAfter');
  await input.evaluate((node) => {
    const text = document.createTreeWalker(node, NodeFilter.SHOW_TEXT).nextNode()!;
    document.getSelection()!.setBaseAndExtent(text, 9, text, 9);
  });
  await input.press('ArrowDown');
  await input.press('End');
  await input.press('Enter');
  const proof = await page.evaluate(async () => {
    const modulePath = '/test/annotation-browser.tsx';
    return (await import(modulePath)).proof();
  });
  expect(proof.writes).toBe(0);
  expect(proof.preparations).toBe(0);
  expect(proof.sends).toHaveLength(0);
  await expect(input).toHaveText('Before @Other agent \nAfter', { useInnerText: true });
});
