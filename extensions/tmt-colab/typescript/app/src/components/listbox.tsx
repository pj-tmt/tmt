import { createPortal } from 'react-dom';
import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
  type HTMLAttributes,
  type Ref,
} from 'react';
import './listbox.css';
import { openModifiedPage, plainPageClick } from '../page-link.js';

export interface ListboxOption<Value extends string> {
  value: Value;
  label: string;
  disabled?: boolean;
  /** A multi-choice list: this option's own state, instead of the one `value`. */
  selected?: boolean;
  /** Navigation results only; selection suggestions remain buttons. */
  href?: string;
  /**
   * The labelled group this option belongs to. Consecutive options of one group share a heading;
   * a list where no option has one keeps its flat markup. The heading is never an option.
   */
  group?: { key: string; label: string };
}

function lastEnabledIndex<Value extends string>(options: readonly ListboxOption<Value>[]) {
  for (let index = options.length - 1; index >= 0; index--) {
    if (!options[index].disabled) return index;
  }
  return -1;
}

/** Consecutive options of one group (or of none) as runs, with where each starts. */
function groupRuns<Value extends string>(options: readonly ListboxOption<Value>[]) {
  const runs: {
    group?: { key: string; label: string };
    start: number;
    options: ListboxOption<Value>[];
  }[] = [];
  options.forEach((option, index) => {
    const last = runs.at(-1);
    if (last && last.group?.key === option.group?.key) last.options.push(option);
    else runs.push({ group: option.group, start: index, options: [option] });
  });
  return runs;
}

/** A single-value, controlled picker. Focus stays on the trigger while the open list is navigated. */
export function Listbox<Value extends string>({
  label,
  options,
  value,
  onChange,
  renderOption,
  disabled = false,
  inputTrigger,
  presentation = 'popup',
  multiple = false,
  popup,
}: {
  label: string;
  options: readonly ListboxOption<Value>[];
  value: Value;
  onChange(value: Value): void;
  renderOption?(option: ListboxOption<Value>): ReactNode;
  disabled?: boolean;
  /** Results share the input's active-option owner without a floating popup. */
  presentation?: 'popup' | 'results';
  /** Every option carries its own selected state and choosing one does not replace the others. */
  multiple?: boolean;
  /** A popup that is not as wide as its trigger (a pill, an icon): its width, and which edge of the
   * trigger it lines up with. */
  popup?: { width: number; align: 'start' | 'end'; className?: string };
  /** Text input owns its value; this same listbox owns option navigation. */
  inputTrigger: {
    open: boolean;
    onOpenChange(open: boolean): void;
    render(props: HTMLAttributes<HTMLElement> & { ref: Ref<HTMLElement> }): ReactNode;
  };
}) {
  const id = useId();
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const setTrigger = useCallback((node: HTMLElement | null) => {
    trigger.current = node;
  }, []);
  const [placement, setPlacement] = useState<{
    left: number;
    top: number;
    width: number;
    maxHeight: number;
  }>();
  const open = inputTrigger.open;
  const setOpen = inputTrigger.onOpenChange;
  const [activeValue, setActiveValue] = useState<Value | null>(null);
  const selectedIndex = options.findIndex((option) => option.value === value && !option.disabled);
  const firstIndex = options.findIndex((option) => !option.disabled);
  const activeIndex = options.findIndex(
    (option) => option.value === activeValue && !option.disabled,
  );
  const focusIndex =
    activeIndex >= 0 ? activeIndex : selectedIndex >= 0 ? selectedIndex : firstIndex;
  const labelId = `${id}-label`;
  const listId = `${id}-list`;
  const optionId = (index: number) =>
    `${id}-option-${presentation === 'results' ? encodeURIComponent(options[index].value) : index}`;

  useEffect(() => {
    if (!open) setActiveValue(null);
  }, [open]);

  useEffect(() => {
    if (presentation === 'results') return;
    if (!open) return;
    const closeOutside = (event: PointerEvent) => {
      if (
        event.target instanceof Node &&
        !root.current?.contains(event.target) &&
        !list.current?.contains(event.target)
      )
        setOpen(false);
    };
    document.addEventListener('pointerdown', closeOutside);
    return () => document.removeEventListener('pointerdown', closeOutside);
  }, [open, setOpen, presentation]);

  useEffect(() => {
    if (presentation === 'results') return;
    if (!list.current) return;
    const node = list.current;
    if (open && trigger.current?.getClientRects().length) node.showPopover();
    else if (node.matches(':popover-open')) node.hidePopover();
    return () => {
      if (node.matches(':popover-open')) node.hidePopover();
    };
  }, [open, presentation]);

  const activeId = open && focusIndex >= 0 ? optionId(focusIndex) : undefined;
  // The first option of a group brings its heading into view with it.
  const startsGroup =
    focusIndex >= 0 && options[focusIndex]?.group?.key !== options[focusIndex - 1]?.group?.key;
  useEffect(() => {
    if (!activeId) return;
    const active = list.current?.querySelector<HTMLElement>(`[id="${activeId}"]`);
    if (startsGroup)
      active?.parentElement
        ?.querySelector('.tmt-listbox-group-label')
        ?.scrollIntoView({ block: 'nearest' });
    active?.scrollIntoView({ block: 'nearest' });
  }, [activeId, startsGroup]);

  useEffect(() => {
    if (presentation === 'results') return;
    if (!open) return;
    const place = () => {
      const box = trigger.current?.getBoundingClientRect();
      if (!box) return;
      const node = list.current;
      if (!box.width || !box.height) {
        if (node?.matches(':popover-open')) node.hidePopover();
        setPlacement(undefined);
        return;
      }
      if (node && !node.matches(':popover-open')) node.showPopover();
      const viewport = window.visualViewport;
      const leftEdge = (viewport?.offsetLeft ?? 0) + 8;
      const rightEdge = (viewport?.offsetLeft ?? 0) + (viewport?.width ?? innerWidth) - 8;
      const lower = (viewport?.offsetTop ?? 0) + (viewport?.height ?? innerHeight) - 8,
        upper = (viewport?.offsetTop ?? 0) + 8;
      const above = Math.max(0, box.top - upper - 4),
        below = Math.max(0, lower - box.bottom - 4);
      const height = Math.min(
        248,
        node ? node.scrollHeight + node.offsetHeight - node.clientHeight : 248,
      );
      const upward = below < height && above > below;
      const maxHeight = Math.max(1, Math.min(248, upward ? above : below));
      const width = Math.min(popup?.width ?? box.width, rightEdge - leftEdge);
      const start = popup?.align === 'end' ? box.right - width : box.left;
      const next = {
        left: Math.max(leftEdge, Math.min(start, rightEdge - width)),
        top: upward ? box.top - Math.min(height, maxHeight) - 2 : box.bottom + 2,
        width,
        maxHeight,
      };
      setPlacement((previous) =>
        previous &&
        Object.keys(next).every(
          (key) => previous[key as keyof typeof next] === next[key as keyof typeof next],
        )
          ? previous
          : next,
      );
    };
    place();
    const observer = new ResizeObserver(place);
    if (trigger.current) observer.observe(trigger.current);
    window.visualViewport?.addEventListener('resize', place);
    window.visualViewport?.addEventListener('scroll', place);
    window.addEventListener('resize', place);
    window.addEventListener('scroll', place, true);
    return () => {
      observer.disconnect();
      window.visualViewport?.removeEventListener('resize', place);
      window.visualViewport?.removeEventListener('scroll', place);
      window.removeEventListener('resize', place);
      window.removeEventListener('scroll', place, true);
    };
  }, [open, options.length, presentation, popup?.width, popup?.align]);

  function openList(index = selectedIndex >= 0 ? selectedIndex : firstIndex) {
    if (disabled || index < 0) return;
    setActiveValue(options[index].value);
    setOpen(true);
  }

  function move(step: -1 | 1) {
    if (presentation === 'results') {
      for (let index = focusIndex + step; index >= 0 && index < options.length; index += step) {
        if (!options[index].disabled) {
          setActiveValue(options[index].value);
          return;
        }
      }
      return;
    }
    for (let offset = 1; offset <= options.length; offset++) {
      const index = (focusIndex + step * offset + options.length) % options.length;
      if (!options[index].disabled) {
        setActiveValue(options[index].value);
        return;
      }
    }
  }

  function choose(option: ListboxOption<Value>) {
    if (option.disabled) return;
    setOpen(false);
    onChange(option.value);
    trigger.current?.focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLElement>) {
    const active = open && focusIndex >= 0 ? options[focusIndex] : undefined;
    if (active?.href && openModifiedPage(event.nativeEvent, active.href)) {
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    if (
      presentation === 'results' &&
      (!event.isTrusted ||
        event.defaultPrevented ||
        event.nativeEvent.isComposing ||
        event.keyCode === 229 ||
        event.altKey ||
        event.metaKey ||
        event.ctrlKey ||
        event.shiftKey ||
        event.key === 'Escape' ||
        event.key === 'Tab' ||
        (event.key === 'Enter' && event.repeat))
    )
      return;
    if (
      (!open && (!['ArrowDown', 'ArrowUp'].includes(event.key) || firstIndex < 0)) ||
      event.key === ' ' ||
      event.nativeEvent.isComposing
    )
      return;
    if (['ArrowDown', 'ArrowUp', 'Home', 'End', 'Enter', 'Tab', 'Escape'].includes(event.key))
      event.stopPropagation();
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
        if (open && focusIndex >= 0) {
          event.preventDefault();
          choose(options[focusIndex]);
        } else setOpen(false);
        break;
    }
  }

  const row = (option: ListboxOption<Value>, index: number) => {
    const Element = option.href ? 'a' : 'button';
    return (
      <Element
        {...(option.href
          ? { href: option.disabled ? undefined : option.href }
          : { type: 'button' as const })}
        role="option"
        className="tmt-listbox-option"
        key={option.value}
        id={optionId(index)}
        aria-selected={
          presentation === 'results'
            ? focusIndex === index
            : (option.selected ?? option.value === value)
        }
        data-active={focusIndex === index}
        disabled={option.href ? undefined : option.disabled}
        aria-disabled={option.href ? option.disabled : undefined}
        tabIndex={-1}
        onPointerMove={() => setActiveValue(option.value)}
        onPointerDown={(event) => {
          if (!option.href || plainPageClick(event)) event.preventDefault();
        }}
        onClick={(event) => {
          if (!event.isTrusted) {
            if (option.href) event.preventDefault();
            return;
          }
          if (option.href) {
            if (!plainPageClick(event)) return;
            event.preventDefault();
          }
          choose(option);
        }}
      >
        {renderOption ? renderOption(option) : option.label}
      </Element>
    );
  };

  const optionList = (
    <div
      ref={list}
      popover={presentation === 'popup' ? 'manual' : undefined}
      className={`tmt-listbox-options ${presentation === 'popup' ? 'tmt-listbox-input-list' : 'tmt-listbox-results-list'} ${popup?.className ?? ''}`}
      style={
        presentation === 'popup'
          ? { ...placement, visibility: placement ? 'visible' : 'hidden' }
          : undefined
      }
      role="listbox"
      aria-multiselectable={multiple || undefined}
      id={listId}
      aria-labelledby={labelId}
      hidden={!open}
    >
      {options.some((option) => option.group)
        ? groupRuns(options).map((run) =>
            run.group ? (
              <div
                role="group"
                className="tmt-listbox-group"
                aria-labelledby={`${id}-group-${encodeURIComponent(run.group.key)}`}
                key={`group:${run.group.key}:${run.start}`}
              >
                <div
                  className="tmt-listbox-group-label"
                  id={`${id}-group-${encodeURIComponent(run.group.key)}`}
                >
                  {run.group.label}
                </div>
                {run.options.map((option, offset) => row(option, run.start + offset))}
              </div>
            ) : (
              run.options.map((option, offset) => row(option, run.start + offset))
            ),
          )
        : options.map(row)}
    </div>
  );

  return (
    <div
      className={`tmt-listbox tmt-listbox-input ${presentation === 'results' ? 'tmt-listbox-results' : ''}`}
      ref={root}
    >
      <span className="tmt-listbox-label" id={labelId}>
        {label}
      </span>
      {inputTrigger.render({
        ref: setTrigger,
        role: 'combobox',
        'aria-label': label,
        'aria-autocomplete': 'list',
        'aria-haspopup': 'listbox',
        'aria-controls': listId,
        'aria-expanded': open,
        'aria-activedescendant': open && focusIndex >= 0 ? optionId(focusIndex) : undefined,
        onKeyDown,
      })}
      {presentation === 'popup'
        ? createPortal(optionList, trigger.current?.closest('dialog') ?? document.body)
        : optionList}
    </div>
  );
}
