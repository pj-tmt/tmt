import { useId, type ComponentPropsWithoutRef, type ReactNode, type SyntheticEvent } from 'react';
import { X, TriangleAlert } from 'lucide-react';
import { BrowserIconAction } from './icon-action.js';
import { useModalDialog, type ModalDismissReason } from './modal-dialog.js';
import { usePresence } from './presence.js';

export type BrowserDialogDismissReason = ModalDismissReason;
export interface BrowserDialogProps extends Omit<
  ComponentPropsWithoutRef<'dialog'>,
  'open' | 'title' | 'children' | 'onCancel' | 'onKeyDown' | 'onPointerDown' | 'onClick' | 'onClose'
> {
  open: boolean;
  title: string;
  size?: 'regular' | 'wide';
  presentation?: 'standard' | 'confirm';
  children: ReactNode;
  /** The actions row under the body (for example Move and Cancel), spaced by a token gap. */
  actions?: ReactNode;
  keepMounted?: boolean;
  onDismiss(reason: BrowserDialogDismissReason, event: SyntheticEvent): void;
  initialFocus?(): HTMLElement | null;
  focusFallback?(): HTMLElement | null;
  /** Navigation may hand focus to its destination instead of the old opener. */
  restoreFocus?: boolean;
}

/** A content-fitting modal surface; values, commands and admission remain with the host. */
export function BrowserDialog({
  open,
  title,
  size = 'regular',
  presentation = 'standard',
  children,
  actions,
  keepMounted = false,
  onDismiss,
  initialFocus,
  focusFallback,
  restoreFocus,
  className = '',
  ...attributes
}: BrowserDialogProps) {
  const label = useId();
  const modal = useModalDialog({ open, onDismiss, initialFocus, focusFallback, restoreFocus });
  const present = usePresence(open, modal.ref);
  if (!present && !keepMounted) return null;
  return (
    <dialog
      {...attributes}
      {...modal}
      className={`tmt-ui-dialog ${className}`}
      data-size={size}
      data-presentation={presentation}
      data-state={open ? 'open' : 'closed'}
      aria-labelledby={label}
    >
      <header className="tmt-ui-dialog-header">
        {presentation === 'confirm' && (
          <div className="tmt-ui-dialog-confirm-mark" aria-hidden="true">
            <TriangleAlert />
          </div>
        )}
        <h2 id={label}>{title}</h2>
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
      <div className="tmt-ui-dialog-body">{children}</div>
      {actions !== undefined && <footer className="tmt-ui-dialog-actions">{actions}</footer>}
    </dialog>
  );
}
