import { useLayoutEffect, useImperativeHandle, useRef } from 'react';
import type { KeyboardEventHandler, ReactNode, Ref } from 'react';
import type { BrowserActionProps } from './action.js';
import { useBusyMark } from './busy-mark.js';
import { bindIconActionTooltip } from './icon-action-tooltip.js';
import { browserUiClasses as c } from './static.js';

export type BrowserIconActionProps = Omit<
  BrowserActionProps,
  'visibleLabel' | 'leadingIcon' | 'variant'
> & {
  variant: Exclude<BrowserActionProps['variant'], 'ghost'>;
  icon: ReactNode;
  /** Optional shortcut help; the button name remains independently sufficient. */
  tooltipLabel?: string;
  ariaKeyShortcuts?: string;
  buttonRef?: Ref<HTMLButtonElement>;
  hasPopup?: 'menu' | 'dialog';
  onKeyDown?: KeyboardEventHandler<HTMLButtonElement>;
} & (
    | { pressed?: boolean; expanded?: never; controls?: never }
    | { pressed?: never; expanded: boolean; controls?: string }
  );

export function BrowserIconAction({
  type,
  label,
  tooltipLabel,
  ariaKeyShortcuts,
  variant,
  icon,
  pressed,
  expanded,
  controls,
  disabled = false,
  disabledReason,
  disabledReasonId,
  busy = false,
  busyMark,
  onActivate,
  buttonRef,
  hasPopup,
  onKeyDown,
}: BrowserIconActionProps) {
  const button = useRef<HTMLButtonElement>(null);
  const tooltip = useRef<HTMLSpanElement>(null);
  const wrapper = useRef<HTMLSpanElement>(null);
  const help = useRef<ReturnType<typeof bindIconActionTooltip> | null>(null);
  const marked = useBusyMark(busy && !disabled);
  useImperativeHandle(buttonRef, () => button.current!, []);
  useLayoutEffect(() => {
    help.current = bindIconActionTooltip(wrapper.current!, button.current!, tooltip.current!);
    return () => {
      help.current?.dispose();
      help.current = null;
    };
  }, []);
  useLayoutEffect(() => {
    help.current?.refresh();
  }, [expanded]);
  if (!label.trim()) throw new Error('Icon action requires a nonempty label');
  if (disabledReason !== undefined && !disabledReasonId)
    throw new Error('Disabled reason requires a stable ID');
  return (
    <>
      <span className={c.iconAction} ref={wrapper}>
        <button
          ref={button}
          className={`${c.action} ${c.iconActionControl}`}
          type={type}
          data-variant={variant}
          disabled={disabled}
          aria-disabled={(busy && !disabled) || undefined}
          aria-label={label}
          aria-keyshortcuts={ariaKeyShortcuts}
          aria-haspopup={hasPopup}
          aria-pressed={pressed}
          aria-expanded={expanded}
          aria-controls={controls}
          aria-busy={busy || undefined}
          aria-describedby={disabledReason !== undefined ? disabledReasonId : undefined}
          onKeyDown={onKeyDown}
          onClick={(event) => {
            // Busy stays focusable; fence its activation and form submission here.
            if (busy) event.preventDefault();
            else if (!disabled) {
              help.current?.dismiss();
              onActivate(event);
            }
          }}
        >
          <span className={c.iconActionIcon} aria-hidden="true">
            {icon}
          </span>
          {busyMark !== undefined && (
            <span className={c.actionMark} aria-hidden="true" data-busy={marked}>
              {busyMark}
            </span>
          )}
        </button>
        <span ref={tooltip} className={c.iconActionTooltip} popover="manual" aria-hidden="true">
          <span className={c.iconActionTooltipLabel}>{tooltipLabel ?? label}</span>
        </span>
      </span>
      {disabledReason !== undefined && (
        <span className={c.fieldDescription} id={disabledReasonId}>
          {disabledReason}
        </span>
      )}
    </>
  );
}
