import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from 'react';
import { Ellipsis } from 'lucide-react';
import { BrowserIconAction } from '@tmt/browser-ui/react';
import './action-menu.css';

export interface ActionMenuItem {
  key: string;
  label: string;
  disabled?: boolean;
  checked?: boolean;
}

/** A row or header action menu. Focus enters the open menu; Escape returns to its trigger. */
export function ActionMenu({
  label,
  items,
  onSelect,
  disabled = false,
  icon,
}: {
  label: string;
  items: readonly ActionMenuItem[];
  onSelect(key: string): void;
  disabled?: boolean;
  /** Header hosts supply their icon/count; row actions retain their ellipsis trigger. */
  icon?: ReactNode;
}) {
  const id = useId();
  const root = useRef<HTMLSpanElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const entries = useRef<(HTMLButtonElement | null)[]>([]);
  const list = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [place, setPlace] = useState<{ right: number; top: number; maxHeight: number }>();
  const enabled = items.flatMap((item, index) => (item.disabled ? [] : [index]));

  // The list drops below the row that owns the trigger (never over that row's own content),
  // right edges aligned with the trigger. When neither side fits, keep the list
  // inside its scrolling container so stationary headers cannot cover its actions.
  useLayoutEffect(() => {
    if (!open) return setPlace(undefined);
    const row = root.current?.offsetParent,
      box = list.current?.getBoundingClientRect(),
      trigger = root.current?.getBoundingClientRect();
    if (!row || !box || !trigger) return;
    const rowBox = row.getBoundingClientRect();
    let top = 0,
      bottom = innerHeight;
    for (let parent = row.parentElement; parent; parent = parent.parentElement) {
      if (/(auto|scroll)/.test(getComputedStyle(parent).overflowY)) {
        const bounds = parent.getBoundingClientRect();
        top = Math.max(top, bounds.top);
        bottom = Math.min(bottom, bounds.bottom);
        break;
      }
    }
    const maxHeight = Math.max(0, bottom - top - 4);
    const height = Math.min(box.height, maxHeight);
    const below = rowBox.bottom + 2;
    const above = rowBox.top - height - 2;
    const y =
      below + height <= bottom - 2
        ? below
        : above >= top + 2
          ? above
          : Math.max(top + 2, bottom - height - 2);
    setPlace({
      // The list is positioned in the row's padding box.
      right: rowBox.left + row.clientLeft + row.clientWidth - trigger.right,
      top: y - rowBox.top - row.clientTop,
      maxHeight,
    });
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const closeOutside = (event: PointerEvent) => {
      if (event.target instanceof Node && !root.current?.contains(event.target)) setOpen(false);
    };
    document.addEventListener('pointerdown', closeOutside);
    return () => document.removeEventListener('pointerdown', closeOutside);
  }, [open]);
  // The first enabled entry takes focus once per opening, when the list is placed and visible.
  const placed = place !== undefined;
  useEffect(() => {
    if (open && placed) entries.current[enabled[0]]?.focus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, placed]);

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
  function onTriggerKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    if (event.key === 'Escape' && open) {
      event.preventDefault();
      event.stopPropagation();
      close(true);
    } else if (event.key === 'ArrowDown' && event.isTrusted) {
      event.preventDefault();
      setOpen(true);
    }
  }
  return (
    <span className={`tmt-action-menu${icon ? ' tmt-action-menu-icon' : ''}`} ref={root}>
      {icon ? (
        <BrowserIconAction
          buttonRef={trigger}
          type="button"
          variant="text"
          label={label}
          icon={icon}
          hasPopup="menu"
          expanded={open}
          controls={open ? id : undefined}
          disabled={disabled || enabled.length === 0}
          onActivate={(event) => {
            if (event.isTrusted) setOpen(!open);
          }}
          onKeyDown={onTriggerKeyDown}
        />
      ) : (
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
          onKeyDown={onTriggerKeyDown}
        >
          <Ellipsis aria-hidden />
        </button>
      )}
      {open && (
        <div
          className="tmt-action-menu-list"
          style={{ ...place, visibility: place ? 'visible' : 'hidden' }}
          ref={list}
          role="menu"
          id={id}
          aria-label={label}
          onKeyDown={onMenuKeyDown}
        >
          {items.map((item, index) => (
            <button
              type="button"
              role={item.checked === undefined ? 'menuitem' : 'menuitemradio'}
              aria-checked={item.checked}
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
              {item.checked && (
                <span className="tmt-action-menu-check" aria-hidden="true">
                  ✓
                </span>
              )}
            </button>
          ))}
        </div>
      )}
    </span>
  );
}
