import { useId, type ComponentPropsWithoutRef, type ReactNode, type SyntheticEvent } from 'react';
import { X } from 'lucide-react';
import { BrowserIconAction } from './icon-action.js';
import { useModalDialog, type ModalDismissReason } from './modal-dialog.js';

export type BrowserDialogDismissReason = ModalDismissReason;
export interface BrowserDialogProps extends Omit<
  ComponentPropsWithoutRef<'dialog'>,
  'open' | 'title' | 'children' | 'onCancel' | 'onKeyDown' | 'onPointerDown' | 'onClick' | 'onClose'
> {
  open: boolean;
  title: string;
  children: ReactNode;
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
  children,
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
  if (!open && !keepMounted) return null;
  return (
    <dialog
      {...attributes}
      {...modal}
      className={`tmt-ui-dialog ${className}`}
      aria-labelledby={label}
    >
      <header className="tmt-ui-dialog-header">
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
    </dialog>
  );
}
