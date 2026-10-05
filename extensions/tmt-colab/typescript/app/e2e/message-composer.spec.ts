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
      const input = page.getByRole('combobox', { name: 'Message to agent', exact: true });
      await expect(input).toHaveValue('@Deterministic agent ');
      const evidence = [];
      for (const position of ['Top', 'Bottom']) {
        await page.getByRole('button', { name: position, exact: true }).click();
        // A fresh edit reopens suggestions after Escape; filling the same value emits no edit.
        await input.fill('');
        await input.fill('@');
        const list = page.getByRole('listbox');
        await expect(list).toBeVisible();
        await expect(list.getByRole('option')).toHaveCount(3);
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
            input: rect(document.querySelector('textarea')!),
            popover: rect(popover),
            portalParent: node.parentElement!.tagName,
            scrollHeight: element.scrollHeight,
            clientHeight: element.clientHeight,
            maxHeight: getComputedStyle(element).maxHeight,
            popoverOverflow: getComputedStyle(popover).overflowY,
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
      for (const geometry of evidence)
        expect(geometry.clientHeight).toBeGreaterThanOrEqual(geometry.scrollHeight);
    });
  }
}

test('recipient selection keeps multiline surrounding text and performs no effects', async ({
  page,
}) => {
  await page.goto(fixture);
  const input = page.getByRole('combobox', { name: 'Message to agent', exact: true });
  await expect(input).toHaveValue('@Deterministic agent ');
  await input.fill('Before @O\nAfter');
  await input.evaluate((node) => (node as HTMLTextAreaElement).setSelectionRange(9, 9));
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
  await expect(input).toHaveValue('Before @Other agent \nAfter');
});
