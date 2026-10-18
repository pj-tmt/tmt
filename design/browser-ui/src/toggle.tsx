import type { MouseEvent, ReactNode } from 'react';
import { browserUiClasses as c } from './static';
export interface BrowserToggleProps {
  pressed: boolean;
  /** Text, or a host node such as an agent mark and name; a node needs `ariaLabel`. */
  label: ReactNode;
  /** The accessible name when the visible label is a node or abbreviates a longer name. */
  ariaLabel?: string;
  disabled?: boolean;
  onActivate: (event: MouseEvent<HTMLButtonElement>) => void;
}
export function BrowserToggle({
  pressed,
  label,
  ariaLabel,
  disabled = false,
  onActivate,
}: BrowserToggleProps) {
  return (
    <button
      className={c.toggle}
      type="button"
      aria-pressed={pressed}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={(event) => {
        if (!disabled) onActivate(event);
      }}
    >
      <span className={c.toggleIndicator} aria-hidden="true">
        {pressed ? '✓' : ''}
      </span>
      <span>{label}</span>
    </button>
  );
}
