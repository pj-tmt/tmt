import type { MouseEvent, ReactNode } from 'react';
import { browserUiClasses as c } from './static';
import type { BrowserActionVariant } from './static';
export interface BrowserActionProps {
  type: 'button' | 'submit' | 'reset';
  label: string;
  variant: BrowserActionVariant;
  disabled?: boolean;
  disabledReason?: string;
  disabledReasonId?: string;
  busy?: boolean;
  busyMark?: ReactNode;
  onActivate: (event: MouseEvent<HTMLButtonElement>) => void;
}
export function BrowserAction({
  type,
  label,
  variant,
  disabled = false,
  disabledReason,
  disabledReasonId,
  busy = false,
  busyMark,
  onActivate,
}: BrowserActionProps) {
  if (disabledReason !== undefined && !disabledReasonId)
    throw new Error('Disabled reason requires a stable ID');
  return (
    <>
      <button
        className={c.action}
        type={type}
        data-variant={variant}
        disabled={disabled || busy}
        aria-busy={busy || undefined}
        aria-describedby={disabledReason !== undefined ? disabledReasonId : undefined}
        onClick={(event) => {
          if (!disabled && !busy) onActivate(event);
        }}
      >
        {busyMark !== undefined && (
          <span className={c.actionMark} aria-hidden="true" data-busy={busy}>
            {busyMark}
          </span>
        )}
        <span className={c.actionLabel}>{label}</span>
      </button>
      {disabledReason !== undefined && (
        <span className={c.fieldDescription} id={disabledReasonId}>
          {disabledReason}
        </span>
      )}
    </>
  );
}
