import { useId, type ComponentPropsWithoutRef, type ReactNode, type SyntheticEvent } from 'react';
import { DrawerHeader } from './drawer-header.js';
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
  /** Host-owned header actions, in DOM and keyboard order immediately before Close. */
  headerActions?: ReactNode;
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
  headerActions,
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
      data-state={open ? 'open' : 'closed'}
      aria-labelledby={label}
    >
      <DrawerHeader
        title={title}
        caption={caption}
        headerActions={headerActions}
        label={label}
        onClose={(event) => onDismiss('close', event)}
      />
      {notice && <div className="tmt-ui-drawer-notice">{notice}</div>}
      <div className="tmt-ui-drawer-body">{children}</div>
      {actions !== undefined && <footer className="tmt-ui-drawer-actions">{actions}</footer>}
      {overlay}
    </dialog>
  );
}
