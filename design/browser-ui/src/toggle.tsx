import type { MouseEvent } from 'react';
import { browserUiClasses as c } from './static';
export interface BrowserToggleProps {
  pressed: boolean;
  label: string;
  disabled?: boolean;
  onActivate: (event: MouseEvent<HTMLButtonElement>) => void;
}
export function BrowserToggle({
  pressed,
  label,
  disabled = false,
  onActivate,
}: BrowserToggleProps) {
  return (
    <button
      className={c.toggle}
      type="button"
      aria-pressed={pressed}
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
