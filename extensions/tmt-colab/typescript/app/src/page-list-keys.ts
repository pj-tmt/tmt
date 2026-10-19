import { useEffect, useRef, type RefObject } from 'react';

/** Where a list shortcut may fire: not while typing, and not above a menu or a dialog (a menu is a
 * modal dialog too). Only the page itself or the list's own section may hold the focus. */
export function admitsListShortcut(event: KeyboardEvent, section: HTMLElement): boolean {
  if (!event.isTrusted || event.defaultPrevented || event.isComposing) return false;
  if (event.ctrlKey || event.metaKey || event.altKey) return false;
  const target = event.target;
  if (!(target instanceof HTMLElement)) return false;
  if (target.closest('textarea, select, [contenteditable], [role="textbox"], dialog')) return false;
  // A checkbox is a control the list's own keys work beside; any other input is typing.
  if (target instanceof HTMLInputElement && !['checkbox', 'radio', 'button'].includes(target.type))
    return false;
  if (target !== document.body && !section.contains(target)) return false;
  return !document.querySelector('dialog[open]');
}

export interface PageListKeyActions {
  /** The list's own section: the keys read its rows, and a hidden list takes none. */
  section: RefObject<HTMLElement | null>;
  /** The list has management, so selecting and archiving mean something. */
  managed: boolean;
  /** Flip the selection of one page. */
  select(pageId: string): void;
  /** Archive the page with this ID, or the selection when none is given. */
  archive(pageId: string | undefined): void;
  /** Open the projects menu of one page. */
  projects(pageId: string): void;
  /** Whether a selection exists to clear with Escape. */
  selecting: boolean;
  clearSelection(): void;
}

/**
 * The list's keys: `/` search, `j`/`k` move between rows (Enter and Ctrl/Cmd+Enter act on the focused
 * link as they always did), `x` select, `e` archive, `l` projects, Escape clears the selection.
 */
export function usePageListKeys(actions: PageListKeyActions) {
  const latest = useRef(actions);
  latest.current = actions;
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const now = latest.current;
      const section = now.section.current;
      if (!section || !section.getClientRects().length || !admitsListShortcut(event, section))
        return;
      const rows = [...section.querySelectorAll<HTMLElement>('.pages > li[data-page-id]')];
      const target = event.target as HTMLElement;
      const current = target.closest<HTMLElement>('li[data-page-id]');
      const at = current ? rows.indexOf(current) : -1;
      switch (event.key) {
        case '/': {
          const field = section.querySelector<HTMLElement>('.page-list-search [contenteditable]');
          if (!field) return;
          field.focus();
          break;
        }
        case 'j':
        case 'k': {
          if (!rows.length) return;
          const next =
            event.key === 'j'
              ? Math.min(at + 1, rows.length - 1)
              : at < 0
                ? rows.length - 1
                : Math.max(at - 1, 0);
          rows[next].querySelector<HTMLElement>('.tmt-ui-list-title a')?.focus();
          break;
        }
        case 'x':
          if (!now.managed || !current?.dataset.pageId || event.repeat) return;
          now.select(current.dataset.pageId);
          break;
        case 'e':
          if (!now.managed || event.repeat) return;
          if (!now.selecting && !current?.dataset.pageId) return;
          now.archive(now.selecting ? undefined : current?.dataset.pageId);
          break;
        case 'l':
          if (!now.managed || !current?.dataset.pageId || event.repeat) return;
          now.projects(current.dataset.pageId);
          break;
        case 'Escape':
          if (!now.selecting) return;
          now.clearSelection();
          break;
        default:
          return;
      }
      event.preventDefault();
    }
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, []);
}
