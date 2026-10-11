import {
  useLayoutEffect,
  useId,
  useRef,
  type ComponentPropsWithoutRef,
  type ReactNode,
  type SyntheticEvent,
} from 'react';
import { X } from 'lucide-react';
import { BrowserIconAction } from './icon-action.js';
import { admitsOverlayEscape, outsideOverlay, restoreOverlayFocus } from './overlay.js';

export type BrowserDrawerDismissReason = 'escape' | 'backdrop' | 'close';
export interface BrowserDrawerProps extends Omit<
  ComponentPropsWithoutRef<'dialog'>,
  'open' | 'title' | 'children' | 'onCancel' | 'onKeyDown' | 'onPointerDown' | 'onClick' | 'onClose'
> {
  open: boolean;
  title: string;
  caption?: ReactNode;
  /** Host-owned status and recovery actions directly below the header. */
  notice?: ReactNode;
  children: ReactNode;
  /** Host presentation over the complete surface, including the shared header. */
  overlay?: ReactNode;
  panel?: string;
  width?: 'regular' | 'wide';
  keepMounted?: boolean;
  className?: string;
  onDismiss(reason: BrowserDrawerDismissReason, event: SyntheticEvent): void;
  focusFallback?(): HTMLElement | null;
}

/** The host owns open state and dismissal admission; native modality owns containment. */
export function BrowserDrawer({
  open,
  title,
  caption,
  notice,
  children,
  overlay,
  panel,
  width = 'regular',
  keepMounted = true,
  className = '',
  onDismiss,
  focusFallback,
  ...attributes
}: BrowserDrawerProps) {
  const label = useId();
  const dialog = useRef<HTMLDialogElement>(null);
  const currentOpen = useRef(open);
  currentOpen.current = open;
  const fallback = useRef(focusFallback);
  fallback.current = focusFallback;
  useLayoutEffect(() => {
    const node = dialog.current;
    if (!open || !node) return;
    const origin = node.ownerDocument.activeElement;
    node.showModal();
    return () => {
      node.close();
      restoreOverlayFocus(origin, fallback.current);
    };
  }, [open]);
  if (!open && !keepMounted) return null;
  return (
    <dialog
      {...attributes}
      ref={dialog}
      className={`tmt-ui-drawer ${className}`}
      data-panel={panel}
      data-width={width}
      aria-labelledby={label}
      onClose={(event) => {
        event.stopPropagation();
        if (currentOpen.current && !event.currentTarget.open) onDismiss('close', event);
      }}
      onCancel={(event) => {
        event.preventDefault();
        event.stopPropagation();
        if (event.isTrusted) onDismiss('escape', event);
      }}
      onKeyDown={(event) => {
        if (event.key !== 'Escape' || event.defaultPrevented) return;
        event.preventDefault();
        event.stopPropagation();
        if (admitsOverlayEscape(event)) onDismiss('escape', event);
      }}
      onPointerDown={(event) => {
        if (
          event.target === event.currentTarget &&
          outsideOverlay(event.currentTarget, event.clientX, event.clientY)
        )
          event.preventDefault();
      }}
      onClick={(event) => {
        if (
          event.target === event.currentTarget &&
          outsideOverlay(event.currentTarget, event.clientX, event.clientY)
        ) {
          event.preventDefault();
          event.stopPropagation();
          if (event.isTrusted) onDismiss('backdrop', event);
        }
      }}
    >
      <header className="tmt-ui-drawer-header">
        <div className="tmt-ui-drawer-heading">
          <h2 id={label}>{title}</h2>
          {caption && <div className="tmt-ui-drawer-caption">{caption}</div>}
        </div>
        <BrowserIconAction
          type="button"
          variant="text"
          label={`Close ${title}`}
          tooltipLabel={`Close ${title} · Esc`}
          ariaKeyShortcuts="Escape"
          icon={<X />}
          onActivate={(event) => {
            if (event.isTrusted) onDismiss('close', event);
          }}
        />
      </header>
      {notice && <div className="tmt-ui-drawer-notice">{notice}</div>}
      <div className="tmt-ui-drawer-body">{children}</div>
      {overlay}
    </dialog>
  );
}
