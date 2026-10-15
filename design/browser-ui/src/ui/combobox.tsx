import { useRef, type ComponentProps, type KeyboardEvent, type MouseEvent, type Ref } from 'react';
import { useCommandState } from 'cmdk';
import type { BrowserChipItem, BrowserChipOption } from '../static.js';
import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from './command.js';
import { Popover, PopoverAnchor, PopoverContent } from './popover.js';
import type { LayerController, LayerFocusOptions } from './layer.js';
import { cn } from './utils.js';

export type SelectionItem = BrowserChipItem;
export type SelectionOption = BrowserChipOption;
export type SelectionEvent = KeyboardEvent<HTMLInputElement> | MouseEvent<HTMLDivElement>;
export interface ComboboxProps extends LayerFocusOptions {
  controller: LayerController;
  label: string;
  query: string;
  onQueryChange(query: string): void;
  options: readonly SelectionOption[];
  value?: string | null;
  onSelect(option: SelectionOption, event: SelectionEvent): void;
  emptyLabel: string;
  placeholder?: string;
  disabled?: boolean;
  readOnly?: boolean;
  busy?: boolean;
  stateLabel?: string;
  inputRef?: Ref<HTMLInputElement>;
  onKeyDown?: ComponentProps<'input'>['onKeyDown'];
  onEmptyBackspace?(event: KeyboardEvent<HTMLInputElement>): void;
  outsidePress?: 'pass-through' | 'dismiss-only';
  className?: string;
}

export function Combobox(props: ComboboxProps) {
  return (
    <Command label={props.label} shouldFilter={false} className={props.className}>
      <SelectionInput {...props} />
    </Command>
  );
}
function SelectionInput({
  controller,
  label,
  query,
  onQueryChange,
  options,
  value,
  onSelect,
  emptyLabel,
  placeholder,
  disabled,
  readOnly,
  busy,
  stateLabel,
  inputRef,
  onKeyDown,
  onEmptyBackspace,
  outsidePress = 'pass-through',
  returnFocus,
  focusFallback,
}: ComboboxProps) {
  const input = useRef<HTMLInputElement | null>(null);
  const anchor = useRef<HTMLDivElement>(null);
  const tabClose = useRef(false);
  const active = useCommandState((state) => state.value);
  const blocked = disabled || readOnly || busy;
  if (blocked && !stateLabel) throw new Error('Blocked selection requires a visible stateLabel');
  const canCommit = () =>
    !blocked && options.find((option) => option.id === active && !option.disabled);
  function keyDown(event: KeyboardEvent<HTMLInputElement>) {
    onKeyDown?.(event);
    if (
      event.defaultPrevented ||
      event.nativeEvent.isComposing ||
      event.keyCode === 229 ||
      !event.nativeEvent.isTrusted ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey ||
      (event.shiftKey && event.key !== 'Tab')
    ) {
      if (event.key === 'Enter') event.stopPropagation();
      return;
    }
    if (event.key === 'Tab') {
      if (!controller.open) return;
      const option = canCommit();
      // Admission precedes any host mutation; the native Tab event remains original.
      if (!controller.requestClose('tab', event.nativeEvent)) {
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      tabClose.current = true;
      if (outsidePress === 'pass-through' && option) onSelect(option, event);
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      event.stopPropagation();
      const option = controller.open && canCommit();
      if (option && controller.requestClose('selection', event.nativeEvent))
        onSelect(option, event);
      return;
    }
    if (event.key === 'Backspace' && !query && !blocked) onEmptyBackspace?.(event);
    if (event.key === 'ArrowDown' && !blocked) controller.requestOpen();
  }
  return (
    <Popover controller={controller}>
      <PopoverAnchor asChild>
        <div ref={anchor} className="tmt:min-w-0">
          <CommandInput asChild value={query} onValueChange={onQueryChange}>
            <input
              aria-label={label}
              aria-expanded={controller.open}
              aria-readonly={readOnly || undefined}
              aria-busy={busy || undefined}
              aria-description={stateLabel}
              disabled={disabled}
              readOnly={readOnly || busy}
              placeholder={placeholder}
              ref={(node) => {
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
              onKeyDown={keyDown}
              onFocus={() => {
                if (!blocked) {
                  tabClose.current = false;
                  controller.requestOpen();
                }
              }}
            />
          </CommandInput>
          {stateLabel && (
            <span
              role="status"
              className="tmt:block tmt:px-3 tmt:py-1 tmt:text-(length:--tmt-foundation-caption-size)"
            >
              {stateLabel}
            </span>
          )}
        </div>
      </PopoverAnchor>
      <PopoverContent
        initialFocus={() => input.current}
        returnFocus={returnFocus ?? (() => input.current)}
        focusFallback={focusFallback}
        align="start"
        className="tmt:p-0"
        onCloseAutoFocus={(event) => {
          if (tabClose.current) {
            event.preventDefault();
            tabClose.current = false;
          }
        }}
        onInteractOutside={(event) => {
          const original = event.detail.originalEvent;
          if (original.target instanceof Node && anchor.current?.contains(original.target)) {
            event.preventDefault();
            return;
          }
          if (outsidePress === 'dismiss-only' && original.type === 'pointerdown') {
            event.preventDefault();
            original.preventDefault();
            controller.requestClose('outside', original);
          }
        }}
      >
        <CommandList aria-label={`${label} options`}>
          <CommandEmpty>{emptyLabel}</CommandEmpty>
          {options.map((option) => (
            <CommandItem
              key={option.id}
              value={option.id}
              disabled={blocked || option.disabled}
              aria-description={option.description}
              onClickCapture={(event) => {
                if (
                  event.defaultPrevented ||
                  !event.nativeEvent.isTrusted ||
                  event.button !== 0 ||
                  event.altKey ||
                  event.ctrlKey ||
                  event.metaKey ||
                  blocked ||
                  option.disabled
                )
                  return;
                if (controller.requestClose('selection', event.nativeEvent))
                  onSelect(option, event);
              }}
            >
              <span
                className={cn(
                  'tmt:min-w-0 tmt:break-words',
                  value === option.id && 'tmt:font-(weight:--tmt-foundation-eyebrow-weight)',
                )}
              >
                {option.label}
              </span>
            </CommandItem>
          ))}
        </CommandList>
      </PopoverContent>
    </Popover>
  );
}
