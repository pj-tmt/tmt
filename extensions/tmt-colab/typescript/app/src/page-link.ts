import type { KeyboardEvent as ReactKeyboardEvent } from 'react';

/** Only an ordinary trusted activation belongs to the in-app navigation owner. */
export function plainPageClick(
  event: Pick<
    MouseEvent,
    'isTrusted' | 'defaultPrevented' | 'button' | 'altKey' | 'ctrlKey' | 'metaKey' | 'shiftKey'
  >,
): boolean {
  return (
    event.isTrusted &&
    !event.defaultPrevented &&
    event.button === 0 &&
    !event.altKey &&
    !event.ctrlKey &&
    !event.metaKey &&
    !event.shiftKey
  );
}

/** A search keeps focus on its input, so modified Enter activates the highlighted href explicitly.
 * These are browser pages only; the plain action remains with the existing navigation owner. */
export function openModifiedPage(event: KeyboardEvent, href: string): boolean {
  if (
    !event.isTrusted ||
    event.defaultPrevented ||
    event.isComposing ||
    event.repeat ||
    event.key !== 'Enter' ||
    event.altKey ||
    !(event.ctrlKey || event.metaKey || event.shiftKey)
  )
    return false;
  window.open(href, '_blank', event.shiftKey ? 'noopener,popup' : 'noopener');
  return true;
}

/** Native anchor activation for page links whose owner keeps plain Enter in this document. */
export function pageLinkKey(event: ReactKeyboardEvent<HTMLAnchorElement>): void {
  if (!openModifiedPage(event.nativeEvent, event.currentTarget.href)) return;
  event.preventDefault();
  event.stopPropagation();
}
