import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useSyncExternalStore,
  type ReactNode,
  type MouseEvent,
} from 'react';
import { ChevronDown, Ellipsis } from 'lucide-react';
import { BrowserIconAction } from './icon-action.js';
import { createMenuController } from './menu-controller.js';
import { browserUiClasses as c, countText } from './static.js';
import type { BrowserCountTone } from './static.js';

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
  /** Without a value: a text trigger (the text and a chevron) in place of the icon action; the
   * accessible name stays `label`. For a command that opens a choice, not a field with a value. */
  triggerText?: string;
  /** With `valueLabel`: `crumb` is a borderless text trigger that inherits the surrounding font
   * (a header crumb); the value truncates with an ellipsis and the chevron stays visible. With
   * `triggerText`: `count` is a count chip (`icon`, the text, `count` and a chevron). */
  variant?: 'field' | 'crumb' | 'count';
  /** A `count` chip's number, boxed after the text; absent when nothing is counted. */
  count?: number;
  /** A `count` chip's tone: `attention` is for work that waits on the reader. */
  tone?: BrowserCountTone;
  description?: ReactNode;
  /** Presentation transitions only; commands and selected values remain host-owned. */
  onOpenChange?(open: boolean): void;
  /** A host shortcut: each new value opens the menu once, as the trigger does. The first value a
   * menu mounts with opens nothing. */
  openRequest?: number;
}

/** Only transient visibility/focus live here; selected values and commands remain host-owned. */
export function BrowserMenu<Key extends string>({
  label,
  items,
  onSelect,
  disabled = false,
  icon,
  valueLabel,
  triggerText,
  variant = 'field',
  count,
  tone = 'neutral',
  description,
  onOpenChange,
  openRequest,
}: BrowserMenuProps<Key>) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const entries = useRef<(HTMLButtonElement | null)[]>([]);
  const latest = useRef({ items, disabled, description, onSelect, onOpenChange });
  latest.current = { items, disabled, description, onSelect, onOpenChange };
  const owner = useRef<ReturnType<
    typeof createMenuController<Key, MouseEvent<HTMLButtonElement>>
  > | null>(null);
  if (!owner.current)
    owner.current = createMenuController({
      items: () => latest.current.items,
      disabled: () => latest.current.disabled,
      hasDescription: () => Boolean(latest.current.description),
      trigger: () => trigger.current,
      surface: () => dialog.current,
      entries: () => entries.current,
      selection: () => latest.current.onSelect,
      onOpenChange: (value) => latest.current.onOpenChange?.(value),
    });
  const controller = owner.current;
  const { open, present } = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
    controller.getSnapshot,
  );
  const requested = useRef(openRequest);
  useEffect(() => {
    const fresh = openRequest !== undefined && openRequest !== requested.current;
    requested.current = openRequest;
    if (fresh) controller.open();
  }, [openRequest, controller]);
  useLayoutEffect(() => controller.attachSurface(), [open, controller]);
  useLayoutEffect(() => controller.refresh(), [items, controller]);
  return (
    <span className="tmt-ui-menu">
      {valueLabel !== undefined || triggerText !== undefined ? (
        <button
          ref={trigger}
          type="button"
          className={
            variant === 'count' ? `tmt-ui-menu-trigger ${c.countChip}` : 'tmt-ui-menu-trigger'
          }
          {...(variant === 'crumb' ? { 'data-variant': 'crumb' } : {})}
          {...(variant === 'count' ? { 'data-tone': tone } : {})}
          aria-label={
            valueLabel !== undefined
              ? `${label}: ${valueLabel}`
              : variant === 'count' && count !== undefined
                ? `${label}: ${count}`
                : label
          }
          aria-haspopup="menu"
          aria-expanded={open}
          aria-controls={open ? id : undefined}
          disabled={!controller.canOpen()}
          onKeyDown={controller.triggerKey}
          onClick={(event) => {
            if (event.isTrusted) {
              controller.open();
            }
          }}
        >
          {variant === 'field' && valueLabel !== undefined && (
            <span className="tmt-ui-menu-field-label">{label}</span>
          )}
          {variant === 'count' && icon}
          <span className="tmt-ui-menu-value">{valueLabel ?? triggerText}</span>
          {variant === 'count' && count !== undefined && (
            <span className={c.count} aria-hidden="true">
              {countText(count)}
            </span>
          )}
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
          disabled={!controller.canOpen()}
          onKeyDown={controller.triggerKey}
          onActivate={(event) => {
            if (event.isTrusted) {
              controller.open();
            }
          }}
        />
      )}
      {present && (
        <dialog
          ref={dialog}
          className="tmt-ui-menu-surface"
          data-state={open ? 'open' : 'closed'}
          tabIndex={-1}
          aria-label={label}
          onClose={controller.nativeClose}
          onCancel={controller.cancel}
          onKeyDown={controller.menuKey}
          onPointerDown={controller.pointerDown}
          onClick={controller.surfaceClick}
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
                  onFocus={() => controller.focused(item.key)}
                  onClick={(event) => controller.select(item.key, event)}
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
