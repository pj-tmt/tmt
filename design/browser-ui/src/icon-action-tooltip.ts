interface TooltipViewport {
  left: number;
  top: number;
  width: number;
  height: number;
}

export function iconActionTooltipPosition(
  anchor: Pick<DOMRect, 'left' | 'right' | 'bottom'>,
  tooltipWidth: number,
  viewport: TooltipViewport,
  gutter: number,
): { left: number; top: number; maxWidth: number; maxHeight: number } {
  const maxWidth = Math.max(0, viewport.width - gutter * 2);
  const width = Math.min(tooltipWidth, maxWidth);
  const leftEdge = viewport.left + gutter;
  const top = Math.max(viewport.top, anchor.bottom);
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
    maxHeight: Math.max(0, viewport.top + viewport.height - gutter - top),
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
    const next = iconActionTooltipPosition(
      box,
      tooltip.getBoundingClientRect().width,
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
