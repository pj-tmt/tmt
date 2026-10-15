import { useId, useLayoutEffect, useRef, type KeyboardEvent, type MouseEvent } from 'react';
import { XIcon } from 'lucide-react';
import { Badge } from './badge.js';
import { Button } from './button.js';
import {
  Combobox,
  type ComboboxProps,
  type SelectionItem,
  type SelectionOption,
  type SelectionEvent,
} from './combobox.js';

export interface ChipProps {
  item: SelectionItem;
  onRemove(item: SelectionItem, event: MouseEvent<HTMLButtonElement>): void;
  readOnly?: boolean;
  busy?: boolean;
  stateLabel?: string;
  removeRef?(node: HTMLButtonElement | null): void;
}
export function Chip({ item, onRemove, readOnly, busy, stateLabel, removeRef }: ChipProps) {
  if ((readOnly || busy || item.disabled) && !stateLabel)
    throw new Error('Blocked chip requires a visible stateLabel');
  const reason = useId();
  const blocked = readOnly || busy || item.disabled;
  return (
    <Badge variant="outline" className="tmt:max-w-full tmt:whitespace-normal tmt:break-words">
      <span>{item.label}</span>
      <Button
        asChild
        ref={removeRef}
        variant="ghost"
        size="icon-sm"
        disabled={item.disabled || busy}
      >
        <button
          type="button"
          aria-label={item.removeLabel}
          aria-describedby={stateLabel ? reason : undefined}
          aria-disabled={blocked || undefined}
          aria-busy={busy || undefined}
          disabled={item.disabled || busy}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') event.stopPropagation();
          }}
          onClick={(event) => {
            if (!blocked && event.nativeEvent.isTrusted && !event.defaultPrevented)
              onRemove(item, event);
          }}
        >
          <XIcon aria-hidden="true" />
        </button>
      </Button>
      {stateLabel && <span id={reason}>{stateLabel}</span>}
    </Badge>
  );
}
export interface ChipsProps extends Omit<
  ComboboxProps,
  'value' | 'onSelect' | 'outsidePress' | 'onEmptyBackspace'
> {
  items: readonly SelectionItem[];
  onAdd(option: SelectionOption, event: SelectionEvent): void;
  onRemove(
    item: SelectionItem,
    event: KeyboardEvent<HTMLInputElement> | MouseEvent<HTMLButtonElement>,
  ): void;
}
/** Host supplies caps through eligible options/state; no recipient/parser/domain state is inferred. */
export function Chips({
  items,
  onAdd,
  onRemove,
  disabled,
  readOnly,
  busy,
  stateLabel,
  inputRef,
  ...props
}: ChipsProps) {
  const input = useRef<HTMLInputElement | null>(null);
  const controls = useRef(new Map<string, HTMLButtonElement>());
  const removalFocus = useRef<{ removed: string; after?: string; before?: string } | null>(null);
  useLayoutEffect(() => {
    const pending = removalFocus.current;
    if (!pending || items.some((item) => item.id === pending.removed)) return;
    removalFocus.current = null;
    const target =
      (pending.after && controls.current.get(pending.after)) ||
      (pending.before && controls.current.get(pending.before)) ||
      input.current ||
      props.focusFallback?.();
    if (target?.isConnected && !target.matches(':disabled') && !target.closest('[inert]'))
      target.focus();
  }, [items, props.focusFallback]);
  function remove(
    item: SelectionItem,
    event: KeyboardEvent<HTMLInputElement> | MouseEvent<HTMLButtonElement>,
  ) {
    if (
      disabled ||
      readOnly ||
      busy ||
      item.disabled ||
      event.defaultPrevented ||
      !event.nativeEvent.isTrusted
    )
      return;
    const index = items.findIndex((candidate) => candidate.id === item.id);
    const next = items.slice(index + 1).find((candidate) => !candidate.disabled);
    const previous = items
      .slice(0, index)
      .reverse()
      .find((candidate) => !candidate.disabled);
    removalFocus.current = { removed: item.id, after: next?.id, before: previous?.id };
    event.preventDefault();
    event.stopPropagation();
    onRemove(item, event);
  }
  return (
    <div data-slot="chips" className="tmt:grid tmt:min-w-0 tmt:gap-2">
      <div aria-label={`${props.label} selected`} className="tmt:flex tmt:flex-wrap tmt:gap-1">
        {items.map((item) => (
          <Chip
            key={item.id}
            item={item}
            readOnly={readOnly || disabled}
            busy={busy}
            stateLabel={item.disabled || readOnly || disabled || busy ? stateLabel : undefined}
            removeRef={(node) => {
              if (node) controls.current.set(item.id, node);
              else controls.current.delete(item.id);
            }}
            onRemove={remove}
          />
        ))}
      </div>
      <Combobox
        {...props}
        disabled={disabled}
        readOnly={readOnly}
        busy={busy}
        stateLabel={stateLabel}
        inputRef={(node) => {
          input.current = node;
          const cleanup = typeof inputRef === 'function' ? inputRef(node) : undefined;
          if (inputRef && typeof inputRef !== 'function') inputRef.current = node;
          return () => {
            input.current = null;
            if (typeof cleanup === 'function') cleanup();
            else if (typeof inputRef === 'function') inputRef(null);
            else if (inputRef) inputRef.current = null;
          };
        }}
        outsidePress="dismiss-only"
        onSelect={onAdd}
        onEmptyBackspace={(event) => {
          const last = [...items].reverse().find((item) => !item.disabled);
          if (last) remove(last, event);
        }}
      />
    </div>
  );
}
