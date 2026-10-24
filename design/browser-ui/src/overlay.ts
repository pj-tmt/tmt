interface OverlayEscapeEvent {
  isTrusted: boolean;
  repeat?: boolean;
  keyCode?: number;
  altKey?: boolean;
  ctrlKey?: boolean;
  metaKey?: boolean;
  shiftKey?: boolean;
  isComposing?: boolean;
  nativeEvent?: { isComposing?: boolean };
}

/** Modified, repeated and composing keys cannot dismiss an actionable layer. */
export function admitsOverlayEscape(event: OverlayEscapeEvent): boolean {
  return (
    event.isTrusted &&
    !event.repeat &&
    !(event.nativeEvent?.isComposing ?? event.isComposing) &&
    event.keyCode !== 229 &&
    !event.altKey &&
    !event.ctrlKey &&
    !event.metaKey &&
    !event.shiftKey
  );
}

export function restoreOverlayFocus(origin: Element | null, fallback?: () => HTMLElement | null) {
  const usable = (node: Element | null): node is HTMLElement =>
    node instanceof HTMLElement &&
    node.isConnected &&
    node.getClientRects().length > 0 &&
    !node.closest('[inert]') &&
    !node.matches(':disabled');
  if (usable(origin)) origin.focus({ preventScroll: true });
  else {
    const node = fallback?.() ?? null;
    if (usable(node)) node.focus({ preventScroll: true });
  }
}

export function outsideOverlay(node: HTMLElement, x: number, y: number): boolean {
  const box = node.getBoundingClientRect();
  return x < box.left || x >= box.right || y < box.top || y >= box.bottom;
}
