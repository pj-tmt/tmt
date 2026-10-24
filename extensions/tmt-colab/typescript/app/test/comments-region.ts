import type { Locator, Page } from '@playwright/test';

/** The Comments surface while it is open: the list and a thread's Discussion view share it. It is the
 * non-modal Comments panel (a column, card or sheet by width); closed, it stays mounted and hidden.
 * Every spec reaches it through here, so a change of container is one edit. */
const commentsSelector = '.tmt-ui-panel[data-panel="comments"]:not([hidden])';

export function commentsRegion(page: Page): Locator {
  return page.locator(commentsSelector);
}

/** The control that closes the Comments region; its name follows the view it shows. */
export function commentsClose(page: Page, view: 'list' | 'discussion'): Locator {
  return commentsRegion(page).getByRole('button', {
    name: view === 'list' ? 'Close Comments' : 'Close Discussion',
    exact: true,
  });
}

/** Whichever surface is open: a modal drawer, or the Comments region. Use it for "no panel is open"
 * and for specs that open a panel by name; use `commentsRegion` when the spec means Comments. */
export function openPanel(page: Page): Locator {
  return page.locator(`.tmt-ui-drawer[open], ${commentsSelector}`);
}
