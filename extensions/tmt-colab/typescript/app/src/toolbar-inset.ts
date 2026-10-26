/**
 * The height above a page frame at scroll 0: the toolbar, plus the row the TMT web shell stacks
 * above its main area on a phone (`--tmt-ui-shell-lead`, 0 elsewhere and outside the shell).
 */
export function toolbarInset(toolbar: HTMLElement | null): number {
  if (!toolbar) return 0;
  const lead = Number.parseFloat(getComputedStyle(toolbar).getPropertyValue('--tmt-ui-shell-lead'));
  return toolbar.offsetHeight + toolbar.getBoundingClientRect().top + (lead > 0 ? lead : 0);
}

/**
 * The height above a page frame at scroll 0, measured at the frame's own host: everything the shell
 * and the page stack over it (the bar, a phone's module tabs and the page's title row, which scrolls
 * away), as a document position that stays put while the page scrolls. Without a host it is the
 * toolbar's.
 */
export function frameInset(host: HTMLElement | null, toolbar: HTMLElement | null): number {
  return host ? host.getBoundingClientRect().top + window.scrollY : toolbarInset(toolbar);
}
