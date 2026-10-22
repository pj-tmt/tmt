import type { MouseEvent, ReactNode, Ref } from 'react';
import { browserUiClasses as c, countText } from './static.js';
import type { BrowserCountTone } from './static.js';

export interface BrowserCountChipProps {
  /** The chip's visible text, such as `Comments`. */
  label: string;
  /** Drawn in a box after the label; absent when nothing is counted. */
  count?: number;
  tone?: BrowserCountTone;
  /** A leading icon or mark; decorative, the label names the chip. */
  icon?: ReactNode;
  disabled?: boolean;
  pressed?: boolean;
  buttonRef?: Ref<HTMLButtonElement>;
  onActivate(event: MouseEvent<HTMLButtonElement>): void;
}

/** A chip that acts on click; for a count that opens a menu, use `BrowserMenu` `variant="count"`. */
export function BrowserCountChip({
  label,
  count,
  tone = 'neutral',
  icon,
  disabled = false,
  pressed,
  buttonRef,
  onActivate,
}: BrowserCountChipProps) {
  return (
    <button
      ref={buttonRef}
      type="button"
      className={c.countChip}
      data-tone={tone}
      aria-label={count === undefined ? label : `${label}: ${count}`}
      aria-pressed={pressed}
      disabled={disabled}
      onClick={(event) => {
        if (event.isTrusted) onActivate(event);
      }}
    >
      {icon}
      <span>{label}</span>
      {count !== undefined && (
        <span className={c.count} aria-hidden="true">
          {countText(count)}
        </span>
      )}
    </button>
  );
}
