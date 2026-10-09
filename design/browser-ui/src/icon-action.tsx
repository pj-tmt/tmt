import { useEffect, useImperativeHandle, useRef, useState } from 'react';
import type { KeyboardEventHandler, ReactNode, Ref } from 'react';
import type { BrowserActionProps } from './action.js';
import { placeIconActionTooltip } from './icon-action-tooltip.js';
import { browserUiClasses as c } from './static.js';

export type BrowserIconActionProps = BrowserActionProps & {
  icon: ReactNode;
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
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const open = (hovered || focused) && !dismissed && !expanded;
  useImperativeHandle(buttonRef, () => button.current!, []);
  useEffect(() => {
    const node = tooltip.current;
    const anchor = button.current;
    if (!open || !node || !anchor) return;
    node.showPopover();
    const dispose = placeIconActionTooltip(anchor, node);
    const escape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || !node.matches(':popover-open')) return;
      event.preventDefault();
      event.stopPropagation();
      setDismissed(true);
    };
    anchor.ownerDocument.addEventListener('keydown', escape, true);
    return () => {
      dispose();
      anchor.ownerDocument.removeEventListener('keydown', escape, true);
      if (node.matches(':popover-open')) node.hidePopover();
    };
  }, [open]);
  if (!label.trim()) throw new Error('Icon action requires a nonempty label');
  if (disabledReason !== undefined && !disabledReasonId)
    throw new Error('Disabled reason requires a stable ID');
  return (
    <>
      <span
        className={c.iconAction}
        onPointerEnter={() => {
          setHovered(true);
          setDismissed(false);
        }}
        onPointerLeave={(event) => {
          if (event.relatedTarget instanceof Node && tooltip.current?.contains(event.relatedTarget))
            return;
          setHovered(false);
        }}
      >
        <button
          ref={button}
          className={`${c.action} ${c.iconActionControl}`}
          type={type}
          data-variant={variant}
          disabled={disabled || busy}
          aria-label={label}
          aria-haspopup={hasPopup}
          aria-pressed={pressed}
          aria-expanded={expanded}
          aria-controls={controls}
          aria-busy={busy || undefined}
          aria-describedby={disabledReason !== undefined ? disabledReasonId : undefined}
          onFocus={(event) => {
            if (event.currentTarget.matches(':focus-visible')) {
              setFocused(true);
              setDismissed(false);
            }
          }}
          onBlur={() => setFocused(false)}
          onKeyDown={onKeyDown}
          onClick={(event) => {
            if (!disabled && !busy) onActivate(event);
          }}
        >
          <span className={c.iconActionIcon} aria-hidden="true">
            {icon}
          </span>
          {busyMark !== undefined && (
            <span className={c.actionMark} aria-hidden="true" data-busy={busy}>
              {busyMark}
            </span>
          )}
        </button>
        <span
          ref={tooltip}
          className={c.iconActionTooltip}
          popover="manual"
          aria-hidden="true"
          onPointerEnter={() => setHovered(true)}
          onPointerLeave={(event) => {
            if (
              event.relatedTarget instanceof Node &&
              button.current?.contains(event.relatedTarget)
            )
              return;
            setHovered(false);
          }}
        >
          <span className={c.iconActionTooltipLabel}>{label}</span>
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
