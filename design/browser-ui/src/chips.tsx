import { useId, useLayoutEffect, useRef, useState } from 'react';
import type { KeyboardEvent, MouseEvent, ReactNode, Ref } from 'react';
import { X } from 'lucide-react';
import { BrowserIconAction } from './icon-action.js';
import { admitsOverlayEscape, outsideOverlay, restoreOverlayFocus } from './overlay.js';
import { claimBrowserPopup, placeBrowserPopup } from './popup.js';
import { usePresence } from './presence.js';
import { browserUiClasses as c } from './static.js';
import type {
  BrowserChipItem as StaticChipItem,
  BrowserChipOption as StaticChipOption,
} from './static.js';

/** A chip value. `content` replaces the label text on screen; `label` stays the fallback. */
export interface BrowserChipItem extends StaticChipItem {
  content?: ReactNode;
}
/** An option value. `content` replaces the label on screen; `label` stays its accessible name. */
export interface BrowserChipOption extends StaticChipOption {
  content?: ReactNode;
}

export interface BrowserChipProps {
  label: string;
  /** Shown instead of `label` when given, such as a host's agent mark and name. */
  content?: ReactNode;
  removeLabel: string;
  disabled?: boolean;
  buttonRef?: Ref<HTMLButtonElement>;
  onRemove(event: MouseEvent<HTMLButtonElement>): void;
}

/** The same item can sit in a controlled list or a host-owned rich editor. */
export function BrowserChip({
  label,
  content,
  removeLabel,
  disabled,
  buttonRef,
  onRemove,
}: BrowserChipProps) {
  return (
    <span
      className={c.chip}
      onKeyDownCapture={(event) => {
        // A rich editor may also own Enter/Space. Keep native remove activation
        // on this control, including when its decorator is a React portal.
        if (
          !event.defaultPrevented &&
          (event.key === 'Enter' || event.key === ' ') &&
          (event.target as HTMLElement).closest('button')
        )
          event.stopPropagation();
      }}
    >
      <span className={c.chipLabel}>{content ?? label}</span>
      <BrowserIconAction
        type="button"
        variant="text"
        label={removeLabel}
        icon={<X />}
        buttonRef={buttonRef}
        disabled={disabled}
        onActivate={onRemove}
      />
    </span>
  );
}

export interface BrowserChipsProps {
  label: string;
  items: readonly BrowserChipItem[];
  options: readonly BrowserChipOption[];
  query: string;
  onQueryChange(query: string): void;
  onAdd(id: string, event: MouseEvent<HTMLButtonElement> | KeyboardEvent<HTMLInputElement>): void;
  onRemove(
    id: string,
    event: MouseEvent<HTMLButtonElement> | KeyboardEvent<HTMLInputElement>,
  ): void;
  addLabel: string;
  searchLabel: string;
  emptyText: string;
  maxItems?: number;
  limitText?: string;
  statusText?: string;
  disabled?: boolean;
  addDisabled?: boolean;
  focusFallback?(): HTMLElement | null;
  onOpenChange?(open: boolean): void;
}

/** Values, filtering and admission stay with the host; only popup visibility/focus live here. */
export function BrowserChips({
  label,
  items,
  options,
  query,
  onQueryChange,
  onAdd,
  onRemove,
  addLabel,
  searchLabel,
  emptyText,
  maxItems,
  limitText,
  statusText,
  disabled = false,
  addDisabled = false,
  focusFallback,
  onOpenChange,
}: BrowserChipsProps) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const buttons = useRef(new Map<string, HTMLButtonElement>());
  const [open, setOpen] = useState(false);
  const present = usePresence(open, dialog);
  const [active, setActive] = useState<string>();
  const visibility = useRef({ reported: false, callback: onOpenChange });
  visibility.current.callback = onOpenChange;
  const capped = maxItems !== undefined && items.length >= maxItems;
  const unavailable = disabled || addDisabled || capped;
  const choices = options.filter((option) => !items.some((item) => item.id === option.id));
  const enabled = choices.filter((option) => !option.disabled);
  const chosen = enabled.find((option) => option.id === active) ?? enabled[0];
  const chosenIndex = choices.findIndex((option) => option.id === chosen?.id);
  function report(value: boolean) {
    if (visibility.current.reported === value) return;
    visibility.current.reported = value;
    visibility.current.callback?.(value);
  }
  function fallback() {
    return (
      [...buttons.current.values()].filter((button) => !button.disabled).at(-1) ??
      focusFallback?.() ??
      null
    );
  }
  function close(refocus = true) {
    dialog.current?.close();
    setOpen(false);
    report(false);
    if (refocus) restoreOverlayFocus(trigger.current, fallback);
  }
  useLayoutEffect(() => {
    const node = dialog.current;
    const anchor = trigger.current;
    if (!open || !node || !anchor) return;
    const release = claimBrowserPopup(anchor.ownerDocument, () => close(false));
    node.showModal();
    report(true);
    const disposePosition = placeBrowserPopup(anchor, node, () => close(false));
    input.current?.focus();
    return () => {
      disposePosition();
      release();
      node.close();
      queueMicrotask(() => {
        if (!node.isConnected) {
          report(false);
          if (anchor.ownerDocument.activeElement === anchor.ownerDocument.body)
            restoreOverlayFocus(anchor, focusFallback);
        }
      });
    };
    // Visibility owns the native lifetime; current host values are read by the handlers.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);
  useLayoutEffect(() => {
    if (open && unavailable) close();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, unavailable]);
  useLayoutEffect(() => {
    if (open && chosenIndex >= 0)
      dialog.current
        ?.querySelector(`#${CSS.escape(`${id}-option-${chosenIndex}`)}`)
        ?.scrollIntoView({ block: 'nearest' });
  }, [open, chosenIndex, id]);
  function add(
    option: BrowserChipOption,
    event: MouseEvent<HTMLButtonElement> | KeyboardEvent<HTMLInputElement>,
  ) {
    if (unavailable || option.disabled || !event.isTrusted) return;
    close();
    onQueryChange('');
    onAdd(option.id, event);
    // At the cap the add trigger becomes disabled after the host commits its value.
    queueMicrotask(() => restoreOverlayFocus(trigger.current, fallback));
  }
  function remove(item: BrowserChipItem, event: MouseEvent<HTMLButtonElement>) {
    if (disabled || item.disabled || !event.isTrusted) return;
    const index = items.findIndex((value) => value.id === item.id);
    const next = items[index + 1]?.id ?? items[index - 1]?.id;
    onRemove(item.id, event);
    queueMicrotask(() =>
      restoreOverlayFocus(next ? (buttons.current.get(next) ?? null) : null, () =>
        trigger.current?.disabled ? (focusFallback?.() ?? null) : trigger.current,
      ),
    );
  }
  function key(event: KeyboardEvent<HTMLInputElement>) {
    if (event.defaultPrevented) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      if (admitsOverlayEscape(event)) close();
      return;
    }
    if (
      !event.isTrusted ||
      event.nativeEvent.isComposing ||
      event.keyCode === 229 ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey
    )
      return;
    if (event.key === 'Tab') {
      close();
      return;
    }
    if (event.shiftKey) return;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      if (!enabled.length) return;
      const at = enabled.findIndex((option) => option.id === chosen?.id);
      setActive(
        enabled[(at + (event.key === 'ArrowDown' ? 1 : -1) + enabled.length) % enabled.length]?.id,
      );
    } else if (event.key === 'Enter') {
      event.preventDefault();
      if (chosen) add(chosen, event);
    } else if (event.key === 'Backspace' && !event.repeat && query === '') {
      const last = items.at(-1);
      if (last && !disabled && !last.disabled) {
        event.preventDefault();
        onRemove(last.id, event);
      }
    }
  }
  return (
    <span className={c.chips} role="group" aria-label={label}>
      {items.map((item) => (
        <BrowserChip
          key={item.id}
          label={item.label}
          content={item.content}
          removeLabel={item.removeLabel}
          disabled={disabled || item.disabled}
          buttonRef={(node) => {
            if (node) buttons.current.set(item.id, node);
            else buttons.current.delete(item.id);
          }}
          onRemove={(event) => remove(item, event)}
        />
      ))}
      {!items.length && <span className={c.chipsStatus}>{emptyText}</span>}
      <button
        type="button"
        className={c.chipsAdd}
        ref={trigger}
        disabled={unavailable}
        aria-label={addLabel}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? `${id}-surface` : undefined}
        aria-describedby={capped && limitText ? `${id}-limit` : undefined}
        onKeyDown={(event) => {
          if (
            !['ArrowDown', 'ArrowUp'].includes(event.key) ||
            !event.isTrusted ||
            unavailable ||
            event.nativeEvent.isComposing ||
            event.keyCode === 229 ||
            event.defaultPrevented ||
            event.altKey ||
            event.ctrlKey ||
            event.metaKey ||
            event.shiftKey
          )
            return;
          event.preventDefault();
          setActive(event.key === 'ArrowUp' ? enabled.at(-1)?.id : enabled[0]?.id);
          setOpen(true);
        }}
        onClick={(event) => {
          if (event.isTrusted && !unavailable) {
            setActive(undefined);
            setOpen(true);
          }
        }}
      >
        <span aria-hidden="true">＋</span>
      </button>
      {capped && limitText && (
        <span className={c.chipsStatus} id={`${id}-limit`}>
          {limitText}
        </span>
      )}
      {!open && statusText && (
        <span className={c.chipsStatus} role="status">
          {statusText}
        </span>
      )}
      {present && (
        <dialog
          ref={dialog}
          id={`${id}-surface`}
          className={c.chipsSurface}
          data-state={open ? 'open' : 'closed'}
          aria-label={addLabel}
          onClose={(event) => {
            event.stopPropagation();
            if (!event.currentTarget.open) close();
          }}
          onCancel={(event) => {
            event.preventDefault();
            event.stopPropagation();
            if (event.isTrusted) close();
          }}
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
          <input
            ref={input}
            className={c.chipsSearch}
            role="combobox"
            aria-label={searchLabel}
            aria-autocomplete="list"
            aria-expanded="true"
            aria-controls={`${id}-list`}
            aria-activedescendant={chosenIndex >= 0 ? `${id}-option-${chosenIndex}` : undefined}
            value={query}
            onChange={(event) => {
              onQueryChange(event.currentTarget.value);
              setActive(undefined);
            }}
            onKeyDown={key}
          />
          {statusText && (
            <span className={c.chipsStatus} role="status">
              {statusText}
            </span>
          )}
          <div id={`${id}-list`} role="listbox" aria-label={searchLabel}>
            {choices.map((option, index) => (
              <button
                key={option.id}
                id={`${id}-option-${index}`}
                type="button"
                className={c.chipsOption}
                role="option"
                aria-selected={chosen?.id === option.id}
                disabled={option.disabled}
                tabIndex={-1}
                aria-label={
                  option.content === undefined
                    ? undefined
                    : option.description
                      ? `${option.label} ${option.description}`
                      : option.label
                }
                onMouseDown={(event) => event.preventDefault()}
                onClick={(event) => add(option, event)}
              >
                <span>{option.content ?? option.label}</span>
                {option.description && <span>{option.description}</span>}
              </button>
            ))}
          </div>
        </dialog>
      )}
    </span>
  );
}
