import type { MouseEvent, ReactNode } from 'react';
import { useBusyMark } from './busy-mark';
import { browserUiClasses as c } from './static';
import type { BrowserActionVariant } from './static';
export interface BrowserActionProps {
  type: 'button' | 'submit' | 'reset';
  label: string;
  visibleLabel?: string;
  variant: BrowserActionVariant;
  disabled?: boolean;
  disabledReason?: string;
  disabledReasonId?: string;
  busy?: boolean;
  busyMark?: ReactNode;
  /** Decorative leading mark; the supplied busyMark replaces it on the existing timing. */
  leadingIcon?: ReactNode;
  onActivate: (event: MouseEvent<HTMLButtonElement>) => void;
}
export function BrowserAction({
  type,
  label,
  visibleLabel,
  variant,
  disabled = false,
  disabledReason,
  disabledReasonId,
  busy = false,
  busyMark,
  leadingIcon,
  onActivate,
}: BrowserActionProps) {
  if (disabledReason !== undefined && !disabledReasonId)
    throw new Error('Disabled reason requires a stable ID');
  const marked = useBusyMark(busy && !disabled);
  const readyMark = leadingIcon === undefined ? busyMark : leadingIcon;
  return (
    <>
      <button
        className={c.action}
        type={type}
        aria-label={visibleLabel === undefined ? undefined : label}
        data-variant={variant}
        disabled={disabled}
        aria-disabled={(busy && !disabled) || undefined}
        aria-busy={busy || undefined}
        aria-describedby={disabledReason !== undefined ? disabledReasonId : undefined}
        onClick={(event) => {
          // Busy stays focusable and readable, so its activation, including a
          // form submission, is fenced here rather than by native disabled.
          if (busy) event.preventDefault();
          else if (!disabled) onActivate(event);
        }}
      >
        {(busyMark !== undefined || leadingIcon !== undefined) && (
          <span
            className={c.actionMark}
            aria-hidden="true"
            data-busy={marked}
            data-leading={leadingIcon !== undefined ? '' : undefined}
          >
            {marked && busyMark !== undefined ? busyMark : readyMark}
          </span>
        )}
        <span className={c.actionLabel}>{visibleLabel ?? label}</span>
      </button>
      {disabledReason !== undefined && (
        <span className={c.fieldDescription} id={disabledReasonId}>
          {disabledReason}
        </span>
      )}
    </>
  );
}
