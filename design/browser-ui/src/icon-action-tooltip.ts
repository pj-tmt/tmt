const activeTooltips = new WeakMap<Document, () => void>();

/** Help has one ephemeral owner per document, independent of menu/dialog state. */
export function claimIconActionTooltip(document: Document, dismiss: () => void): () => void {
  activeTooltips.get(document)?.();
  activeTooltips.set(document, dismiss);
  return () => {
    if (activeTooltips.get(document) === dismiss) activeTooltips.delete(document);
  };
}

interface TooltipViewport {
  left: number;
  top: number;
  width: number;
  height: number;
}

export function iconActionTooltipPosition(
  anchor: Pick<DOMRect, 'left' | 'right' | 'top' | 'bottom'>,
  tooltipWidth: number,
  tooltipHeight: number,
  viewport: TooltipViewport,
  gutter: number,
): { left: number; top: number; maxWidth: number; maxHeight: number } {
  const maxWidth = Math.max(0, viewport.width - gutter * 2);
  const width = Math.min(tooltipWidth, maxWidth);
  const leftEdge = viewport.left + gutter;
  const belowTop = Math.max(viewport.top, anchor.bottom);
  const below = Math.max(0, viewport.top + viewport.height - gutter - belowTop);
  const above = Math.max(0, anchor.top - viewport.top - gutter);
  const flipped = tooltipHeight > below && above > below;
  const maxHeight = flipped ? above : below;
  const top = flipped ? anchor.top - Math.min(tooltipHeight, maxHeight) : belowTop;
  return {
    left: Math.max(
      leftEdge,
      Math.min(
        (anchor.left + anchor.right - width) / 2,
        viewport.left + viewport.width - gutter - width,
      ),
    ),
    top,
    maxWidth,
    maxHeight,
  };
}

// Placement lasts only as long as the caller's visible manual popover.
export function placeIconActionTooltip(anchor: HTMLElement, tooltip: HTMLElement): () => void {
  const view = anchor.ownerDocument.defaultView;
  if (!view) return () => {};
  const viewport = view.visualViewport;
  const place = () => {
    const box = anchor.getBoundingClientRect();
    if (!anchor.isConnected || !box.width || !box.height) {
      if (tooltip.matches(':popover-open')) tooltip.hidePopover();
      return;
    }
    const gutter = Number.parseFloat(view.getComputedStyle(tooltip).paddingTop) || 0;
    const bounds = {
      left: viewport?.offsetLeft ?? 0,
      top: viewport?.offsetTop ?? 0,
      width: viewport?.width ?? view.innerWidth,
      height: viewport?.height ?? view.innerHeight,
    };
    tooltip.style.maxWidth = `${Math.max(0, bounds.width - gutter * 2)}px`;
    const tooltipBox = tooltip.getBoundingClientRect();
    const next = iconActionTooltipPosition(
      box,
      tooltipBox.width,
      // Read the full content height even after a previous placement constrained it.
      Math.max(tooltipBox.height, tooltip.scrollHeight),
      bounds,
      gutter,
    );
    tooltip.style.left = `${next.left}px`;
    tooltip.style.top = `${next.top}px`;
    tooltip.style.maxHeight = `${next.maxHeight}px`;
  };
  place();
  const observer = new ResizeObserver(place);
  observer.observe(anchor);
  observer.observe(tooltip);
  view.addEventListener('resize', place);
  view.addEventListener('scroll', place, true);
  viewport?.addEventListener('resize', place);
  viewport?.addEventListener('scroll', place);
  return () => {
    observer.disconnect();
    view.removeEventListener('resize', place);
    view.removeEventListener('scroll', place, true);
    viewport?.removeEventListener('resize', place);
    viewport?.removeEventListener('scroll', place);
  };
}

const escapeFocus = new WeakSet<Document>();
function guardEscapeFocus(document: Document) {
  escapeFocus.add(document);
  const clear = () => {
    escapeFocus.delete(document);
    document.removeEventListener('keydown', nextKey, true);
  };
  const nextKey = (event: KeyboardEvent) => {
    if (event.key !== 'Escape') clear();
  };
  document.addEventListener('keydown', nextKey, true);
  document.defaultView?.requestAnimationFrame(clear);
}

/** Internal binding shared by icon renderers; activation and its busy fence stay with the caller. */
export function bindIconActionTooltip(
  wrapper: HTMLElement,
  button: HTMLElement,
  tooltip: HTMLElement,
) {
  const document = button.ownerDocument;
  const view = document.defaultView!;
  let hovered = false;
  let focused = false;
  let dismissed = false;
  let disposed = false;
  let expanded = button.getAttribute('aria-expanded') === 'true';
  let returnGuard = false;
  let returnFrame: number | undefined;
  let release: (() => void) | undefined;
  let position: (() => void) | undefined;
  const close = () => {
    position?.();
    position = undefined;
    release?.();
    release = undefined;
    document.removeEventListener('keydown', escape, true);
    if (tooltip.matches(':popover-open')) tooltip.hidePopover();
  };
  const dismiss = () => {
    dismissed = true;
    close();
  };
  const escape = (event: KeyboardEvent) => {
    if (event.key !== 'Escape' || !tooltip.matches(':popover-open')) return;
    dismiss();
    guardEscapeFocus(document);
  };
  const paint = () => {
    const open =
      !disposed &&
      (hovered || focused) &&
      !dismissed &&
      !expanded &&
      button.isConnected &&
      button.getClientRects().length > 0;
    if (!open) {
      close();
      return;
    }
    if (tooltip.matches(':popover-open')) return;
    release = claimIconActionTooltip(document, dismiss);
    tooltip.showPopover();
    position = placeIconActionTooltip(button, tooltip);
    document.addEventListener('keydown', escape, true);
  };
  const clearReturnGuard = () => {
    returnGuard = false;
    document.removeEventListener('keydown', returnKey, true);
    if (returnFrame !== undefined) view.cancelAnimationFrame(returnFrame);
    returnFrame = undefined;
  };
  const returnKey = (event: KeyboardEvent) => {
    if (event.key !== 'Escape') clearReturnGuard();
  };
  const refresh = () => {
    const next = button.getAttribute('aria-expanded') === 'true';
    if (expanded && !next) {
      clearReturnGuard();
      returnGuard = true;
      document.addEventListener('keydown', returnKey, true);
      returnFrame = view.requestAnimationFrame(clearReturnGuard);
    }
    expanded = next;
    if (expanded) dismissed = true;
    paint();
  };
  const move = () => {
    if (!hovered) {
      hovered = true;
      dismissed = false;
    }
    refresh();
  };
  const leave = () => {
    hovered = false;
    refresh();
  };
  const focus = () => {
    if (!button.matches(':focus-visible')) return;
    focused = true;
    expanded = button.getAttribute('aria-expanded') === 'true';
    dismissed = expanded || returnGuard || escapeFocus.has(document);
    paint();
  };
  const blur = () => {
    focused = false;
    refresh();
  };
  wrapper.addEventListener('pointermove', move);
  wrapper.addEventListener('pointerleave', leave);
  button.addEventListener('focus', focus);
  button.addEventListener('blur', blur);
  return {
    refresh,
    dismiss,
    dispose() {
      if (disposed) return;
      disposed = true;
      close();
      clearReturnGuard();
      wrapper.removeEventListener('pointermove', move);
      wrapper.removeEventListener('pointerleave', leave);
      button.removeEventListener('focus', focus);
      button.removeEventListener('blur', blur);
    },
  };
}
