import {
  useLayoutEffect,
  useRef,
  type SyntheticEvent,
  type KeyboardEvent,
  type PointerEvent,
  type MouseEvent,
} from 'react';
import { admitsOverlayEscape, outsideOverlay, restoreOverlayFocus } from './overlay.js';

export type ModalDismissReason = 'escape' | 'backdrop' | 'close';

/** Shared native lifetime; the host alone admits dismissal and owns open state. */
export function useModalDialog({
  open,
  onDismiss,
  focusFallback,
  initialFocus,
  restoreFocus = true,
}: {
  open: boolean;
  onDismiss(reason: ModalDismissReason, event: SyntheticEvent): void;
  focusFallback?(): HTMLElement | null;
  initialFocus?(): HTMLElement | null;
  restoreFocus?: boolean;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const current = useRef({ open, onDismiss, focusFallback, initialFocus, restoreFocus });
  current.current = { open, onDismiss, focusFallback, initialFocus, restoreFocus };
  useLayoutEffect(() => {
    const node = dialog.current;
    if (!open || !node) return;
    const origin = node.ownerDocument.activeElement;
    node.showModal();
    const target = current.current.initialFocus?.();
    if (target && node.contains(target)) target.focus({ preventScroll: true });
    return () => {
      node.close();
      if (current.current.restoreFocus) restoreOverlayFocus(origin, current.current.focusFallback);
    };
  }, [open]);
  const outside = (event: {
    target: EventTarget | null;
    currentTarget: HTMLDialogElement;
    clientX: number;
    clientY: number;
  }) =>
    event.target === event.currentTarget &&
    outsideOverlay(event.currentTarget, event.clientX, event.clientY);
  return {
    ref: dialog,
    onClose(event: SyntheticEvent<HTMLDialogElement>) {
      event.stopPropagation();
      if (current.current.open && !event.currentTarget.open)
        current.current.onDismiss('close', event);
    },
    onCancel(event: SyntheticEvent<HTMLDialogElement>) {
      event.preventDefault();
      event.stopPropagation();
      if (event.isTrusted) current.current.onDismiss('escape', event);
    },
    onKeyDown(event: KeyboardEvent<HTMLDialogElement>) {
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      event.preventDefault();
      event.stopPropagation();
      if (admitsOverlayEscape(event)) current.current.onDismiss('escape', event);
    },
    onPointerDown(event: PointerEvent<HTMLDialogElement>) {
      if (outside(event)) event.preventDefault();
    },
    onClick(event: MouseEvent<HTMLDialogElement>) {
      if (!outside(event)) return;
      event.preventDefault();
      event.stopPropagation();
      if (event.isTrusted) current.current.onDismiss('backdrop', event);
    },
  };
}
