/**
 * The height above a page frame at scroll 0: the toolbar, plus the row the TMT web shell stacks
 * above its main area on a phone (`--tmt-ui-shell-lead`, 0 elsewhere and outside the shell).
 */
export function toolbarInset(toolbar: HTMLElement | null): number {
  if (!toolbar) return 0;
  const lead = Number.parseFloat(getComputedStyle(toolbar).getPropertyValue('--tmt-ui-shell-lead'));
  return toolbar.offsetHeight + toolbar.getBoundingClientRect().top + (lead > 0 ? lead : 0);
}
