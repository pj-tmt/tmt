import type { Locator } from '@playwright/test';

/** How many leading characters of a heading's text fit its box before the ellipsis starts. */
export function fittingCharacters(heading: Locator): Promise<number> {
  return heading.evaluate((node) => {
    const style = getComputedStyle(node);
    const context = document.createElement('canvas').getContext('2d')!;
    context.font = style.font;
    const text = node.textContent ?? '';
    let fitting = 0;
    while (
      fitting < text.length &&
      context.measureText(text.slice(0, fitting + 1)).width <= node.clientWidth
    )
      fitting += 1;
    return fitting;
  });
}
