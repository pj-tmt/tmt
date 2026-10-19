import {
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type MouseEvent,
  type KeyboardEvent,
} from 'react';
import { ChevronDown, Ellipsis } from 'lucide-react';
import { BrowserIconAction } from './icon-action.js';
import { admitsOverlayEscape, outsideOverlay, restoreOverlayFocus } from './overlay.js';
import { claimBrowserPopup, placeBrowserPopup } from './popup.js';

export interface BrowserMenuItem<Key extends string = string> {
  key: Key;
  label: string;
  content?: ReactNode;
  disabled?: boolean;
  disabledReason?: string;
  checked?: boolean;
  role?: 'menuitem' | 'menuitemradio' | 'menuitemcheckbox';
  /** A rule above this row, to group the rows that follow it. */
  separated?: boolean;
}
export interface BrowserMenuProps<Key extends string = string> {
  label: string;
  items: readonly BrowserMenuItem<Key>[];
  onSelect(key: Key, event: MouseEvent<HTMLButtonElement>): void;
  disabled?: boolean;
  icon?: ReactNode;
  /** A field's selected value; omitting it uses the icon action trigger. */
  valueLabel?: string;
  /** With `valueLabel`: `crumb` is a borderless text trigger that inherits the surrounding font
   * (a header crumb); the value truncates with an ellipsis and the chevron stays visible. */
  variant?: 'field' | 'crumb';
  description?: ReactNode;
  /** Presentation transitions only; commands and selected values remain host-owned. */
  onOpenChange?(open: boolean): void;
}

/** Only transient visibility/focus live here; selected values and commands remain host-owned. */
export function BrowserMenu<Key extends string>({
  label,
  items,
  onSelect,
  disabled = false,
  icon,
  valueLabel,
  variant = 'field',
  description,
  onOpenChange,
}: BrowserMenuProps<Key>) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const entries = useRef<(HTMLButtonElement | null)[]>([]);
  const [open, setOpen] = useState(false);
  const first = useRef<'first' | 'last'>('first');
  const search = useRef({ text: '', at: 0 });
  const visibility = useRef({ reported: false, callback: onOpenChange });
  visibility.current.callback = onOpenChange;
  function report(open: boolean) {
    if (visibility.current.reported === open) return;
    visibility.current.reported = open;
    visibility.current.callback?.(open);
  }
  const enabled = items.flatMap((item, index) => (item.disabled ? [] : [index]));
  function close(refocus = true) {
    dialog.current?.close();
    report(false);
    setOpen(false);
    if (refocus) restoreOverlayFocus(trigger.current);
  }
  useLayoutEffect(() => {
    const node = dialog.current;
    const anchor = trigger.current;
    if (!open || !node || !anchor) return;
    search.current = { text: '', at: 0 };
    const document = anchor.ownerDocument;
    const release = claimBrowserPopup(document, () => close(false));
    node.showModal();
    report(true);
    const disposePosition = placeBrowserPopup(anchor, node, () => close(false));
    const entry = entries.current[first.current === 'last' ? enabled.at(-1)! : enabled[0]];
    (entry ?? node).focus();
    return () => {
      disposePosition();
      release();
      node.close();
      // StrictMode replays effects while the node remains connected. Only a
      // real removal owes a final transition; ordinary close already reported it.
      queueMicrotask(() => {
        if (!node.isConnected) report(false);
      });
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
          {...(variant === 'crumb' ? { 'data-variant': 'crumb' } : {})}
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
          {variant === 'field' && <span className="tmt-ui-menu-field-label">{label}</span>}
          <span className="tmt-ui-menu-value">{valueLabel}</span>
          <ChevronDown className="tmt-ui-icon" aria-hidden="true" />
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
                  {...(item.separated ? { 'data-separated': 'true' } : {})}
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
