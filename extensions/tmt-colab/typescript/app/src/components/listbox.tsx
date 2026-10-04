import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import './listbox.css';

export interface ListboxOption<Value extends string> {
  value: Value;
  label: string;
  disabled?: boolean;
}

function lastEnabledIndex<Value extends string>(options: readonly ListboxOption<Value>[]) {
  for (let index = options.length - 1; index >= 0; index--) {
    if (!options[index].disabled) return index;
  }
  return -1;
}

/** A single-value, controlled picker. Focus stays on the trigger while the open list is navigated. */
export function Listbox<Value extends string>({
  label,
  options,
  value,
  onChange,
  renderOption,
  disabled = false,
}: {
  label: string;
  options: readonly ListboxOption<Value>[];
  value: Value;
  onChange(value: Value): void;
  renderOption?(option: ListboxOption<Value>): ReactNode;
  disabled?: boolean;
}) {
  const id = useId();
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [activeValue, setActiveValue] = useState<Value | null>(null);
  const selected = options.find((option) => option.value === value);
  const selectedIndex = options.findIndex((option) => option.value === value && !option.disabled);
  const firstIndex = options.findIndex((option) => !option.disabled);
  const activeIndex = options.findIndex(
    (option) => option.value === activeValue && !option.disabled,
  );
  const focusIndex =
    activeIndex >= 0 ? activeIndex : selectedIndex >= 0 ? selectedIndex : firstIndex;
  const labelId = `${id}-label`;
  const valueId = `${id}-value`;
  const listId = `${id}-list`;

  useEffect(() => {
    if (!open) return;
    const closeOutside = (event: PointerEvent) => {
      if (event.target instanceof Node && !root.current?.contains(event.target)) setOpen(false);
    };
    document.addEventListener('pointerdown', closeOutside);
    return () => document.removeEventListener('pointerdown', closeOutside);
  }, [open]);

  useEffect(() => {
    if (!open || focusIndex < 0) return;
    list.current?.children[focusIndex]?.scrollIntoView({ block: 'nearest' });
  }, [open, focusIndex]);

  function openList(index = selectedIndex >= 0 ? selectedIndex : firstIndex) {
    if (disabled || index < 0) return;
    setActiveValue(options[index].value);
    setOpen(true);
  }

  function move(step: -1 | 1) {
    for (let index = focusIndex + step; index >= 0 && index < options.length; index += step) {
      if (!options[index].disabled) {
        setActiveValue(options[index].value);
        return;
      }
    }
  }

  function choose(option: ListboxOption<Value>) {
    if (option.disabled) return;
    setOpen(false);
    if (option.value !== value) onChange(option.value);
    trigger.current?.focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    switch (event.key) {
      case 'ArrowDown':
      case 'ArrowUp':
        event.preventDefault();
        if (open) move(event.key === 'ArrowDown' ? 1 : -1);
        else openList();
        break;
      case 'Home':
      case 'End': {
        event.preventDefault();
        const index = event.key === 'Home' ? firstIndex : lastEnabledIndex(options);
        if (index >= 0) openList(index);
        break;
      }
      case 'Enter':
      case ' ':
        event.preventDefault();
        if (open && focusIndex >= 0) choose(options[focusIndex]);
        else openList();
        break;
      case 'Escape':
        if (open) {
          event.preventDefault();
          setOpen(false);
        }
        break;
      case 'Tab':
        setOpen(false);
        break;
    }
  }

  return (
    <div className="tmt-listbox" ref={root}>
      <span className="tmt-listbox-label" id={labelId}>
        {label}
      </span>
      <button
        ref={trigger}
        type="button"
        className="tmt-listbox-trigger"
        role="combobox"
        id={`${id}-trigger`}
        aria-labelledby={`${labelId} ${valueId}`}
        aria-haspopup="listbox"
        aria-controls={listId}
        aria-expanded={open}
        aria-activedescendant={open && focusIndex >= 0 ? `${id}-option-${focusIndex}` : undefined}
        disabled={disabled || firstIndex < 0}
        onClick={() => (open ? setOpen(false) : openList())}
        onKeyDown={onKeyDown}
      >
        <span id={valueId}>{selected?.label ?? 'Choose an option'}</span>
        <span className="tmt-listbox-chevron" aria-hidden="true">
          ▾
        </span>
      </button>
      <div
        ref={list}
        className="tmt-listbox-options"
        role="listbox"
        id={listId}
        aria-labelledby={labelId}
        hidden={!open}
      >
        {options.map((option, index) => (
          <button
            type="button"
            role="option"
            className="tmt-listbox-option"
            key={option.value}
            id={`${id}-option-${index}`}
            aria-selected={option.value === value}
            data-active={focusIndex === index}
            disabled={option.disabled}
            tabIndex={-1}
            onPointerMove={() => setActiveValue(option.value)}
            onClick={() => choose(option)}
          >
            {renderOption ? renderOption(option) : option.label}
          </button>
        ))}
      </div>
    </div>
  );
}
