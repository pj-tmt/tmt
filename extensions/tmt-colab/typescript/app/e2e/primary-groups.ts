import { expect, type Page } from '@playwright/test';

/**
 * One loud action per group (#4475): `primary` is the accent fill, so a group of actions has at
 * most one enabled primary and the rest are `secondary` or `text`. Two enabled primary actions are
 * crowded when they share a form, dialog, notice or a panel that marks itself `data-action-group`,
 * or sit within a gap of each other (a group of buttons in a row or a column). Each crowded pair comes back as the two accessible names.
 */
export function crowdedPrimaries(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const gap = 24;
    const container =
      'form, dialog, [role="dialog"], [role="group"], [role="toolbar"], .tmt-ui-notice';
    // A panel that marks itself is one group, whatever forms and rows it holds.
    const groupOf = (node: HTMLElement) =>
      node.closest('[data-action-group]') ?? node.closest(container);
    const shown = (node: HTMLElement) =>
      node.getClientRects().length > 0 && getComputedStyle(node).visibility !== 'hidden';
    const primaries = Array.from(
      document.querySelectorAll<HTMLButtonElement>('.tmt-ui-action[data-variant="primary"]'),
    ).filter((node) => !node.disabled && shown(node));
    const name = (node: HTMLElement) =>
      node.getAttribute('aria-label') ?? node.textContent?.trim() ?? '';
    const crowded: string[] = [];
    primaries.forEach((a, index) => {
      for (const b of primaries.slice(index + 1)) {
        const [boxA, boxB] = [a.getBoundingClientRect(), b.getBoundingClientRect()];
        const across = Math.max(boxA.left - boxB.right, boxB.left - boxA.right);
        const down = Math.max(boxA.top - boxB.bottom, boxB.top - boxA.bottom);
        const near = (across < 0 && down <= gap) || (down < 0 && across <= gap);
        const shared = groupOf(a);
        if (near || (shared !== null && shared === groupOf(b)))
          crowded.push(`${name(a)} + ${name(b)}`);
      }
    });
    return crowded;
  });
}

/** The surface in view holds no crowded primary actions. */
export async function expectOnePrimaryPerGroup(page: Page, surface: string) {
  expect(await crowdedPrimaries(page), `${surface}: crowded primary actions`).toEqual([]);
}
