import { useId, type ComponentPropsWithoutRef, type ReactNode, type SyntheticEvent } from 'react';
import { X } from 'lucide-react';
import { BrowserIconAction } from './icon-action.js';
import { useModalDialog } from './modal-dialog.js';

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
  /** The actions row under the body (for example Move and Cancel), spaced by a token gap. */
  actions?: ReactNode;
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
  actions,
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
  const modal = useModalDialog({ open, onDismiss, focusFallback });
  if (!open && !keepMounted) return null;
  return (
    <dialog
      {...attributes}
      {...modal}
      className={`tmt-ui-drawer ${className}`}
      data-panel={panel}
      data-width={width}
      aria-labelledby={label}
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
      {actions !== undefined && <footer className="tmt-ui-drawer-actions">{actions}</footer>}
      {overlay}
    </dialog>
  );
}
