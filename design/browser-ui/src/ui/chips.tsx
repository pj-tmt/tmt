import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  type KeyboardEvent,
  type MouseEvent,
} from 'react';
import { XIcon } from 'lucide-react';
import { Badge } from './badge.js';
import { assignRefs } from './utils.js';
import { Button } from './button.js';
import {
  Combobox,
  type ComboboxProps,
  type SelectionOption,
  type SelectionEvent,
} from './combobox.js';

export interface SelectionItem {
  id: string;
  label: string;
  removeLabel: string;
  disabled?: boolean;
}

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
    // The remove control is a full-height square segment flush with the badge's inner edges: its
    // fill meets them, a left rule separates it, and focus recolours its own border.
    <Badge
      variant="outline"
      className="tmt:max-w-full tmt:whitespace-normal tmt:break-words tmt:p-0 tmt:gap-0 tmt:items-stretch"
    >
      <span className="tmt:self-center tmt:px-2 tmt:py-0.5">{item.label}</span>
      {stateLabel && (
        <span id={reason} className="tmt:self-center tmt:pe-2">
          {stateLabel}
        </span>
      )}
      <Button
        asChild
        ref={removeRef}
        variant="ghost"
        size="icon-sm"
        className="tmt:h-auto tmt:min-h-8 tmt:border-l-border tmt:focus-visible:outline-none"
        disabled={item.disabled || busy}
      >
        <button
          type="button"
          aria-label={item.removeLabel}
          aria-describedby={stateLabel ? reason : undefined}
          aria-disabled={blocked || undefined}
          aria-busy={busy || undefined}
          disabled={item.disabled || busy}
          onClickCapture={(event) => {
            if (readOnly) {
              event.preventDefault();
              event.stopPropagation();
            }
          }}
          onKeyDownCapture={(event) => {
            if (readOnly && (event.key === 'Enter' || event.key === ' ')) {
              event.preventDefault();
              event.stopPropagation();
            }
          }}
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
    </Badge>
  );
}
export interface ChipsProps extends Omit<
  ComboboxProps,
  'value' | 'onSelect' | 'outsidePress' | 'commitOnTab' | 'onEmptyBackspace'
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
  const open = props.controller.open;
  const focusFallback = props.focusFallback;
  useEffect(() => {
    if (!open) return;
    const field = input.current;
    const view = field?.ownerDocument.defaultView;
    const viewport = view?.visualViewport;
    if (!field || !view || !viewport) return;
    let pendingFrame: number | undefined;
    const queueReveal = () => {
      if (pendingFrame !== undefined) view.cancelAnimationFrame(pendingFrame);
      pendingFrame = view.requestAnimationFrame(() => {
        pendingFrame = undefined;
        if (!field.isConnected || field.ownerDocument.activeElement !== field) return;
        const rect = field.getBoundingClientRect();
        if (
          rect.left < viewport.offsetLeft ||
          rect.top < viewport.offsetTop ||
          rect.right > viewport.offsetLeft + viewport.width ||
          rect.bottom > viewport.offsetTop + viewport.height
        )
          field.scrollIntoView({ block: 'nearest', inline: 'nearest' });
      });
    };
    viewport.addEventListener('resize', queueReveal);
    return () => {
      viewport.removeEventListener('resize', queueReveal);
      if (pendingFrame !== undefined) view.cancelAnimationFrame(pendingFrame);
    };
  }, [open]);
  useLayoutEffect(() => {
    const pending = removalFocus.current;
    if (!pending || items.some((item) => item.id === pending.removed)) return;
    removalFocus.current = null;
    // A successor the same host update disabled or hid does not end the search;
    // the host fallback is asked only when no earlier candidate took focus.
    for (const candidate of [
      () => (pending.after ? controls.current.get(pending.after) : undefined),
      () => (pending.before ? controls.current.get(pending.before) : undefined),
      () => input.current,
      () => focusFallback?.(),
    ]) {
      const target = candidate();
      if (!target?.isConnected || target.matches(':disabled') || target.closest('[inert]'))
        continue;
      target.focus();
      if (target.ownerDocument.activeElement === target) return;
    }
  }, [items, focusFallback]);
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
        inputRef={(node) => assignRefs(node, input, inputRef)}
        outsidePress="dismiss-only"
        commitOnTab={false}
        onSelect={onAdd}
        onEmptyBackspace={(event) => {
          const last = [...items].reverse().find((item) => !item.disabled);
          if (last) remove(last, event);
        }}
      />
    </div>
  );
}
