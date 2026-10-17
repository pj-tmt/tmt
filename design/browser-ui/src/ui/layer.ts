import {
  createContext,
  useCallback,
  useContext,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type Ref,
} from 'react';
import { assignRefs } from './utils.js';

export type LayerCloseReason =
  | 'escape'
  | 'outside'
  | 'scrim'
  | 'close'
  | 'selection'
  | 'tab'
  | 'programmatic';
export interface LayerCloseRequest {
  reason: LayerCloseReason;
  originalEvent?: Event;
}
export interface LayerControllerOptions {
  open: boolean;
  onOpenChange(open: boolean): void;
  canClose?(request: LayerCloseRequest): boolean;
}
export interface LayerController {
  open: boolean;
  requestOpen(): void;
  requestClose(reason?: LayerCloseReason, originalEvent?: Event): boolean;
}

/** Host state remains authoritative. Raw open=false/unmount bypasses this admission. */
export function useLayerController(options: LayerControllerOptions): LayerController {
  const current = useRef(options);
  const closing = useRef(false);
  if (current.current.open !== options.open) closing.current = false;
  current.current = options;
  const requestClose = useCallback(
    (reason: LayerCloseReason = 'programmatic', originalEvent?: Event) => {
      if (!current.current.open || closing.current) return false;
      if (current.current.canClose?.({ reason, originalEvent }) === false) return false;
      closing.current = true;
      try {
        current.current.onOpenChange(false);
      } catch (error) {
        closing.current = false;
        throw error;
      }
      return true;
    },
    [],
  );
  const requestOpen = useCallback(() => {
    if (!current.current.open) current.current.onOpenChange(true);
  }, []);
  return { open: options.open, requestOpen, requestClose };
}

// This context only carries the nearest host's admission; Radix owns its layer stack.
export const LayerAdmission = createContext<LayerController | undefined>(undefined);
export function useLayerAdmission(): LayerController | undefined {
  return useContext(LayerAdmission);
}

export function useMenuLayerHandoff({
  menu,
  layer,
  trigger,
}: {
  menu: LayerController;
  layer: LayerController;
  trigger(): HTMLElement | null;
}) {
  const transferring = useRef(false);
  const returning = useRef(false);
  return {
    onSelect(event: Event) {
      // Radix's item default cannot bypass the host's admission or open twice.
      event.preventDefault();
      if (!menu.requestClose('selection', event)) return;
      transferring.current = true;
      returning.current = true;
      layer.requestOpen();
    },
    onCloseAutoFocus(event: Event) {
      if (!transferring.current) return;
      transferring.current = false;
      event.preventDefault();
    },
    returnFocus() {
      if (!returning.current) return null;
      returning.current = false;
      return trigger();
    },
  };
}

export interface LayerFocusOptions {
  initialFocus?(): HTMLElement | null;
  returnFocus?(): HTMLElement | null;
  focusFallback?(): HTMLElement | null;
}

/** Shared with Dialog/Sheet/Popover/Menu; never overrides a consumed Radix focus event. */
export function focusLayerTarget(target: HTMLElement | null | undefined): boolean {
  if (
    !target?.isConnected ||
    target === target.ownerDocument.body ||
    target === target.ownerDocument.documentElement ||
    !target.getClientRects().length ||
    target.closest('[inert]') ||
    target.matches(':disabled')
  )
    return false;
  target.focus({ preventScroll: true });
  return target.ownerDocument.activeElement === target;
}

export function useLayerFocus(options: LayerFocusOptions, titleEntry = false) {
  const origin = useRef<HTMLElement | null>(null);
  const captureOrigin = useCallback((surface: HTMLElement) => {
    const active = surface.ownerDocument.activeElement;
    // Ref replay must not replace a persistent origin with focus already inside.
    if (
      active instanceof HTMLElement &&
      active !== surface.ownerDocument.body &&
      active !== surface.ownerDocument.documentElement &&
      !surface.contains(active)
    )
      origin.current = active;
  }, []);
  return {
    captureOrigin,
    onOpenAutoFocus(event: Event) {
      if (event.defaultPrevented) return;
      const surface = event.currentTarget;
      if (!(surface instanceof HTMLElement)) return;
      captureOrigin(surface);
      const target =
        options.initialFocus?.() ??
        (titleEntry ? surface.querySelector<HTMLElement>('[data-layer-title]') : null);
      if (focusLayerTarget(target)) event.preventDefault();
    },
    onCloseAutoFocus(event: Event) {
      if (event.defaultPrevented) return;
      // A pointer-opened menu can have no focused opener. Leave its trigger return
      // to Radix unless the host supplied an explicit persistent target.
      if (!options.returnFocus && !origin.current) return;
      if (
        focusLayerTarget(options.returnFocus?.() ?? origin.current) ||
        focusLayerTarget(options.focusFallback?.())
      )
        event.preventDefault();
      origin.current = null;
    },
  };
}

export function admitLayerDismiss(
  event: Event,
  controller: LayerController | undefined,
  reason: LayerCloseReason,
): void {
  if (event.defaultPrevented || !controller) return;
  // One host request handles both accepted and vetoed paths; no second Radix close.
  event.preventDefault();
  const detail: unknown = 'detail' in event ? event.detail : undefined;
  const original =
    typeof detail === 'object' &&
    detail !== null &&
    'originalEvent' in detail &&
    detail.originalEvent instanceof Event
      ? detail.originalEvent
      : event;
  controller.requestClose(reason, original);
}

/** Observe only a mounted Radix surface. Geometry uses the viewport, styling uses tokens. */
export function observeLayerViewport(
  node: HTMLElement,
  update: (style: CSSProperties) => void,
): () => void {
  const view = node.ownerDocument.defaultView;
  if (!view) return () => {};
  const viewport = view.visualViewport;
  const measure = () => {
    const spacing = Number.parseFloat(
      view.getComputedStyle(node).getPropertyValue('--tmt-foundation-spacing-unit'),
    );
    const gutter = Number.isFinite(spacing) ? spacing * 2 : 0;
    const left = viewport?.offsetLeft ?? 0;
    const top = viewport?.offsetTop ?? 0;
    const width = viewport?.width ?? view.innerWidth;
    const height = viewport?.height ?? view.innerHeight;
    update({
      '--tmt-layer-left': `${left}px`,
      '--tmt-layer-top': `${top}px`,
      '--tmt-layer-width': `${Math.max(0, width)}px`,
      '--tmt-layer-height': `${Math.max(0, height)}px`,
      '--tmt-layer-available-width': `${Math.max(0, width - gutter * 2)}px`,
      '--tmt-layer-available-height': `${Math.max(0, height - gutter * 2)}px`,
      '--tmt-layer-gutter': `${gutter}px`,
    } as CSSProperties);
  };
  measure();
  view.addEventListener('resize', measure);
  view.addEventListener('scroll', measure, { capture: true });
  viewport?.addEventListener('resize', measure);
  viewport?.addEventListener('scroll', measure);
  return () => {
    view.removeEventListener('resize', measure);
    view.removeEventListener('scroll', measure, { capture: true });
    viewport?.removeEventListener('resize', measure);
    viewport?.removeEventListener('scroll', measure);
  };
}

export function useLayerSurface(
  forwardedRef?: Ref<HTMLDivElement>,
  onMount?: (node: HTMLElement) => void,
) {
  const [node, setNode] = useState<HTMLDivElement | null>(null);
  const [viewportStyle, setViewportStyle] = useState<CSSProperties>({});
  const ref = useCallback(
    (element: HTMLDivElement | null) => {
      if (element) onMount?.(element);
      return assignRefs(element, setNode, forwardedRef);
    },
    [forwardedRef, onMount],
  );
  useLayoutEffect(() => (node ? observeLayerViewport(node, setViewportStyle) : undefined), [node]);
  return { ref, viewportStyle };
}
