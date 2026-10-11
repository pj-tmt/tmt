import {
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type MouseEvent,
  type KeyboardEvent,
} from 'react';
import { Ellipsis } from 'lucide-react';
import { BrowserIconAction } from './icon-action.js';
import { admitsOverlayEscape, outsideOverlay, restoreOverlayFocus } from './overlay.js';
import { browserMenuPosition } from './menu-position.js';

export interface BrowserMenuItem<Key extends string = string> {
  key: Key;
  label: string;
  content?: ReactNode;
  disabled?: boolean;
  disabledReason?: string;
  checked?: boolean;
  role?: 'menuitem' | 'menuitemradio' | 'menuitemcheckbox';
}
export interface BrowserMenuProps<Key extends string = string> {
  label: string;
  items: readonly BrowserMenuItem<Key>[];
  onSelect(key: Key, event: MouseEvent<HTMLButtonElement>): void;
  disabled?: boolean;
  icon?: ReactNode;
  /** A field's selected value; omitting it uses the icon action trigger. */
  valueLabel?: string;
  description?: ReactNode;
}
const activeMenus = new WeakMap<Document, () => void>();

/** Only transient visibility/focus live here; selected values and commands remain host-owned. */
export function BrowserMenu<Key extends string>({
  label,
  items,
  onSelect,
  disabled = false,
  icon,
  valueLabel,
  description,
}: BrowserMenuProps<Key>) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const entries = useRef<(HTMLButtonElement | null)[]>([]);
  const [open, setOpen] = useState(false);
  const first = useRef<'first' | 'last'>('first');
  const search = useRef({ text: '', at: 0 });
  const enabled = items.flatMap((item, index) => (item.disabled ? [] : [index]));
  function close(refocus = true) {
    dialog.current?.close();
    setOpen(false);
    if (refocus) restoreOverlayFocus(trigger.current);
  }
  useLayoutEffect(() => {
    const node = dialog.current;
    const anchor = trigger.current;
    if (!open || !node || !anchor) return;
    search.current = { text: '', at: 0 };
    const document = anchor.ownerDocument;
    activeMenus.get(document)?.();
    const dismiss = () => close(false);
    activeMenus.set(document, dismiss);
    node.showModal();
    const view = document.defaultView!;
    const viewport = view.visualViewport;
    const place = () => {
      if (!anchor.isConnected || !anchor.getClientRects().length) return close(false);
      const bounds = {
        left: viewport?.offsetLeft ?? 0,
        top: viewport?.offsetTop ?? 0,
        width: viewport?.width ?? view.innerWidth,
        height: viewport?.height ?? view.innerHeight,
      };
      node.style.maxWidth = `${Math.max(0, bounds.width - 16)}px`;
      const box = node.getBoundingClientRect();
      const position = browserMenuPosition(
        anchor.getBoundingClientRect(),
        box.width,
        Math.max(box.height, node.scrollHeight + node.offsetHeight - node.clientHeight),
        bounds,
      );
      Object.assign(node.style, {
        left: `${position.left}px`,
        top: `${position.top}px`,
        maxHeight: `${position.maxHeight}px`,
      });
    };
    place();
    const observer = new ResizeObserver(place);
    observer.observe(anchor);
    observer.observe(node);
    view.addEventListener('resize', place);
    view.addEventListener('scroll', place, true);
    viewport?.addEventListener('resize', place);
    viewport?.addEventListener('scroll', place);
    const entry = entries.current[first.current === 'last' ? enabled.at(-1)! : enabled[0]];
    (entry ?? node).focus();
    return () => {
      observer.disconnect();
      view.removeEventListener('resize', place);
      view.removeEventListener('scroll', place, true);
      viewport?.removeEventListener('resize', place);
      viewport?.removeEventListener('scroll', place);
      if (activeMenus.get(document) === dismiss) activeMenus.delete(document);
      node.close();
    };
    // Visibility owns the lifetime; checked values are read from the host on each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);
  function triggerKey(event: KeyboardEvent<HTMLButtonElement>) {
    if (
      !['ArrowDown', 'ArrowUp'].includes(event.key) ||
      !event.isTrusted ||
      event.nativeEvent.isComposing
    )
      return;
    event.preventDefault();
    first.current = event.key === 'ArrowUp' ? 'last' : 'first';
    setOpen(true);
  }
  function menuKey(event: KeyboardEvent<HTMLDialogElement>) {
    if (event.defaultPrevented) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      if (admitsOverlayEscape(event)) close();
      return;
    }
    if (event.nativeEvent.isComposing || event.altKey || event.ctrlKey || event.metaKey) return;
    if (event.key === 'Tab') {
      close();
      return;
    }
    const at = enabled.indexOf(
      entries.current.findIndex((node) => node === node?.ownerDocument.activeElement),
    );
    let next: number | undefined;
    if (event.key === 'ArrowDown') next = enabled[(at + 1) % enabled.length];
    else if (event.key === 'ArrowUp') next = enabled[(at - 1 + enabled.length) % enabled.length];
    else if (event.key === 'Home') next = enabled[0];
    else if (event.key === 'End') next = enabled.at(-1);
    else if (event.key.length === 1 && event.key !== ' ') {
      const now = performance.now();
      search.current.text =
        (now - search.current.at > 700 ? '' : search.current.text) + event.key.toLocaleLowerCase();
      search.current.at = now;
      next = [...enabled.slice(at + 1), ...enabled.slice(0, at + 1)].find((index) =>
        items[index].label.toLocaleLowerCase().startsWith(search.current.text),
      );
    } else return;
    event.preventDefault();
    if (next !== undefined) entries.current[next]?.focus();
  }
  return (
    <span className="tmt-ui-menu">
      {valueLabel !== undefined ? (
        <button
          ref={trigger}
          type="button"
          className="tmt-ui-menu-trigger"
          aria-label={`${label}: ${valueLabel}`}
          aria-haspopup="menu"
          aria-expanded={open}
          aria-controls={open ? id : undefined}
          disabled={disabled || (!enabled.length && !description)}
          onKeyDown={triggerKey}
          onClick={(event) => {
            if (event.isTrusted) {
              first.current = 'first';
              setOpen(true);
            }
          }}
        >
          <span className="tmt-ui-menu-field-label">{label}</span>
          <span>{valueLabel}</span>
          <span aria-hidden="true">⌄</span>
        </button>
      ) : (
        <BrowserIconAction
          buttonRef={trigger}
          type="button"
          variant="text"
          label={label}
          icon={icon ?? <Ellipsis />}
          hasPopup="menu"
          expanded={open}
          controls={open ? id : undefined}
          disabled={disabled || (!enabled.length && !description)}
          onKeyDown={triggerKey}
          onActivate={(event) => {
            if (event.isTrusted) {
              first.current = 'first';
              setOpen(true);
            }
          }}
        />
      )}
      {open && (
        <dialog
          ref={dialog}
          className="tmt-ui-menu-surface"
          tabIndex={-1}
          aria-label={label}
          onClose={(event) => {
            event.stopPropagation();
            if (event.currentTarget === dialog.current && !event.currentTarget.open) close();
          }}
          onCancel={(event) => {
            event.preventDefault();
            event.stopPropagation();
            if (event.isTrusted) close();
          }}
          onKeyDown={menuKey}
          onPointerDown={(event) => {
            if (
              event.target === event.currentTarget &&
              outsideOverlay(event.currentTarget, event.clientX, event.clientY)
            )
              event.preventDefault();
          }}
          onClick={(event) => {
            if (
              event.target === event.currentTarget &&
              outsideOverlay(event.currentTarget, event.clientX, event.clientY)
            ) {
              event.preventDefault();
              event.stopPropagation();
              if (event.isTrusted) close();
            }
          }}
        >
          <div
            role="menu"
            id={id}
            tabIndex={-1}
            aria-label={label}
            aria-describedby={description ? `${id}-description` : undefined}
          >
            {description && (
              <div className="tmt-ui-menu-description" id={`${id}-description`}>
                {description}
              </div>
            )}
            {items.map((item, index) => (
              <div key={item.key}>
                <button
                  type="button"
                  className="tmt-ui-menu-item"
                  role={item.role ?? (item.checked === undefined ? 'menuitem' : 'menuitemradio')}
                  aria-checked={item.checked}
                  aria-describedby={item.disabledReason ? `${id}-reason-${index}` : undefined}
                  disabled={item.disabled}
                  tabIndex={-1}
                  ref={(node) => {
                    entries.current[index] = node;
                  }}
                  onClick={(event) => {
                    if (event.isTrusted && !item.disabled) {
                      close();
                      onSelect(item.key, event);
                    }
                  }}
                >
                  <span>{item.content ?? item.label}</span>
                  {item.checked && <span aria-hidden="true">✓</span>}
                </button>
                {item.disabledReason && (
                  <span className="tmt-ui-menu-reason" id={`${id}-reason-${index}`}>
                    {item.disabledReason}
                  </span>
                )}
              </div>
            ))}
          </div>
        </dialog>
      )}
    </span>
  );
}
