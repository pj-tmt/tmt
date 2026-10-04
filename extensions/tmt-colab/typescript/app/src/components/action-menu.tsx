import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';
import './action-menu.css';

export interface ActionMenuItem {
  key: string;
  label: string;
  disabled?: boolean;
}

/** A text-button menu for per-row actions. Focus moves into the open menu; Esc returns it to the trigger. */
export function ActionMenu({
  label,
  items,
  onSelect,
  disabled = false,
}: {
  label: string;
  items: readonly ActionMenuItem[];
  onSelect(key: string): void;
  disabled?: boolean;
}) {
  const id = useId();
  const root = useRef<HTMLSpanElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const entries = useRef<(HTMLButtonElement | null)[]>([]);
  const [open, setOpen] = useState(false);
  const enabled = items.flatMap((item, index) => (item.disabled ? [] : [index]));

  useEffect(() => {
    if (!open) return;
    entries.current[enabled[0]]?.focus();
    const closeOutside = (event: PointerEvent) => {
      if (event.target instanceof Node && !root.current?.contains(event.target)) setOpen(false);
    };
    document.addEventListener('pointerdown', closeOutside);
    return () => document.removeEventListener('pointerdown', closeOutside);
    // The first enabled entry takes focus once per opening.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  function close(refocus: boolean) {
    setOpen(false);
    if (refocus) trigger.current?.focus();
  }
  function onMenuKeyDown(event: KeyboardEvent<HTMLElement>) {
    const at = entries.current.findIndex((node) => node === document.activeElement);
    const step = (by: number) => {
      const position = enabled.indexOf(at);
      return enabled[(position + by + enabled.length) % enabled.length];
    };
    let next: number | undefined;
    if (event.key === 'ArrowDown') next = step(1);
    else if (event.key === 'ArrowUp') next = step(-1);
    else if (event.key === 'Home') next = enabled[0];
    else if (event.key === 'End') next = enabled[enabled.length - 1];
    else if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      close(true);
      return;
    } else if (event.key === 'Tab') {
      close(false);
      return;
    } else return;
    event.preventDefault();
    if (next !== undefined) entries.current[next]?.focus();
  }
  return (
    <span className="tmt-action-menu" ref={root}>
      <button
        ref={trigger}
        type="button"
        className="tmt-action-menu-trigger"
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        disabled={disabled || enabled.length === 0}
        onClick={(event) => {
          if (event.isTrusted) setOpen(!open);
        }}
        onKeyDown={(event) => {
          if (event.key === 'Escape' && open) {
            event.preventDefault();
            event.stopPropagation();
            close(true);
          } else if (event.key === 'ArrowDown' && event.isTrusted) {
            event.preventDefault();
            setOpen(true);
          }
        }}
      >
        <span aria-hidden="true">⋯</span>
      </button>
      {open && (
        <div
          className="tmt-action-menu-list"
          role="menu"
          id={id}
          aria-label={label}
          onKeyDown={onMenuKeyDown}
        >
          {items.map((item, index) => (
            <button
              type="button"
              role="menuitem"
              className="tmt-action-menu-item"
              key={item.key}
              ref={(node) => {
                entries.current[index] = node;
              }}
              tabIndex={-1}
              disabled={item.disabled}
              onClick={(event) => {
                if (!event.isTrusted) return;
                close(true);
                onSelect(item.key);
              }}
            >
              {item.label}
            </button>
          ))}
        </div>
      )}
    </span>
  );
}
