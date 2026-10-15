import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import { UiTheme } from '../src/ui/theme.js';
import {
  Chip,
  Chips,
  Combobox,
  Command,
  CommandInput,
  CommandItem,
  CommandList,
} from '../src/ui/selection.js';

// Capture only this component's effect boundary for deterministic host-resource
// tests. Server-rendered component tests below retain the real React hooks.
const resources = vi.hoisted(() => ({
  capture: false,
  refs: [] as { current: unknown }[],
  effects: [] as (() => void | (() => void))[],
}));
vi.mock('react', async (original) => {
  const actual = await original<typeof import('react')>();
  return {
    ...actual,
    useRef<T>(value: T) {
      if (!resources.capture) return actual.useRef(value);
      const ref = { current: value };
      resources.refs.push(ref);
      return ref;
    },
    useEffect(...args: Parameters<typeof actual.useEffect>) {
      if (resources.capture) resources.effects.push(args[0]);
      else actual.useEffect(...args);
    },
    useLayoutEffect(...args: Parameters<typeof actual.useLayoutEffect>) {
      if (!resources.capture) actual.useLayoutEffect(...args);
    },
  };
});
afterEach(() => {
  resources.capture = false;
  resources.refs = [];
  resources.effects = [];
});

const controller = { open: false, requestOpen: vi.fn(), requestClose: vi.fn(() => true) };
const option = { id: 'one', label: 'Choice one' };
const props = {
  controller,
  label: 'Choose',
  query: 'Kept query',
  onQueryChange: vi.fn(),
  options: [option],
  onSelect: vi.fn(),
  emptyLabel: 'No choices',
};
// Server markup is semantic/state evidence only. Browser tests own events/focus/portals.
it('renders the controlled query once with honest closed combobox semantics and no server portal', () => {
  const html = renderToStaticMarkup(
    <UiTheme>
      <Combobox {...props} />
    </UiTheme>,
  );
  expect(html.match(/role="combobox"/g)).toHaveLength(1);
  expect(html).toContain('value="Kept query"');
  expect(html).toContain('aria-expanded="false"');
  expect(html).not.toContain('role="option"');
  expect(props.onSelect).not.toHaveBeenCalled();
});
it('requires visible blocked-state words and preserves query while busy/read-only', () => {
  for (const state of [{ busy: true }, { readOnly: true }, { disabled: true }]) {
    expect(() =>
      renderToStaticMarkup(
        <UiTheme>
          <Combobox {...props} {...state} />
        </UiTheme>,
      ),
    ).toThrow('visible stateLabel');
    const html = renderToStaticMarkup(
      <UiTheme>
        <Combobox {...props} {...state} stateLabel="Retained: unavailable" />
      </UiTheme>,
    );
    expect(html).toContain('Retained: unavailable');
    expect(html).toContain('value="Kept query"');
    if (state.busy) expect(html).toContain('aria-busy="true"');
    if (state.readOnly) expect(html).toContain('aria-readonly="true"');
  }
});
it('keeps chip identity/name and blocked removal reason visible without inventing an operation', () => {
  const remove = vi.fn();
  const item = { id: 'stable', label: 'Kept chip', removeLabel: 'Remove kept chip' };
  const html = renderToStaticMarkup(
    <Chip item={item} onRemove={remove} readOnly stateLabel="Read-only" />,
  );
  expect(html).toContain('Kept chip');
  expect(html).toContain('aria-label="Remove kept chip"');
  expect(html).toContain('aria-disabled="true"');
  expect(html).toContain('Read-only');
  expect(remove).not.toHaveBeenCalled();
  expect(() => renderToStaticMarkup(<Chip item={item} onRemove={remove} busy />)).toThrow(
    'visible stateLabel',
  );
});
it('composes host-supplied selected items and query without adding product state', () => {
  const { onSelect, ...rest } = props;
  const html = renderToStaticMarkup(
    <UiTheme>
      <Chips
        {...rest}
        items={[{ id: 'one', label: 'Chosen', removeLabel: 'Remove chosen' }]}
        onAdd={onSelect}
        onRemove={vi.fn()}
      />
    </UiTheme>,
  );
  expect(html).toContain('Chosen');
  expect(html).toContain('Kept query');
  expect(html).toContain('aria-label="Remove chosen"');
});
it('retains cmdk native listbox/option/input relationships in the raw command composition', () => {
  const html = renderToStaticMarkup(
    <Command label="Commands">
      <CommandInput />
      <CommandList>
        <CommandItem value="one">One</CommandItem>
      </CommandList>
    </Command>,
  );
  expect(html).toContain('role="listbox"');
  expect(html).toContain('role="option"');
  expect(html).toContain('aria-controls=');
  expect(html).toContain('aria-disabled="false"');
});

it('reveals only the focused out-of-viewport input once per resize and cleans listener/pending frame', () => {
  resources.capture = true;
  Chips({
    ...props,
    controller: { ...controller, open: true },
    items: [],
    onAdd: vi.fn(),
    onRemove: vi.fn(),
  });
  const viewport = Object.assign(new EventTarget(), {
    width: 320,
    height: 420,
    offsetLeft: 0,
    offsetTop: 0,
  });
  const frames = new Map<number, FrameRequestCallback>();
  let sequence = 0;
  const view = {
    visualViewport: viewport,
    requestAnimationFrame: vi.fn((callback: FrameRequestCallback) => {
      frames.set(++sequence, callback);
      return sequence;
    }),
    cancelAnimationFrame: vi.fn((id: number) => {
      frames.delete(id);
    }),
  };
  const owner = { activeElement: null as unknown, defaultView: view };
  let rect = { left: 0, top: 430, right: 120, bottom: 470 };
  const field = {
    isConnected: true,
    ownerDocument: owner,
    getBoundingClientRect: () => rect,
    scrollIntoView: vi.fn(),
  };
  owner.activeElement = field;
  resources.refs[0].current = field;
  const cleanup = resources.effects[0]();
  expect(cleanup).toBeTypeOf('function');
  const runFrame = () => {
    const entry = [...frames.entries()][0];
    expect(entry).toBeDefined();
    frames.delete(entry[0]);
    entry[1](0);
  };
  viewport.dispatchEvent(new Event('resize'));
  viewport.dispatchEvent(new Event('resize'));
  expect(frames.size).toBe(1);
  runFrame();
  expect(field.scrollIntoView.mock.calls).toEqual([[{ block: 'nearest', inline: 'nearest' }]]);
  expect(frames.size).toBe(0); // no reveal/polling loop
  rect = { left: 0, top: 10, right: 120, bottom: 50 };
  viewport.dispatchEvent(new Event('resize'));
  runFrame();
  expect(field.scrollIntoView).toHaveBeenCalledTimes(1);
  owner.activeElement = {};
  rect = { left: 0, top: 430, right: 120, bottom: 470 };
  viewport.dispatchEvent(new Event('resize'));
  runFrame();
  expect(field.scrollIntoView).toHaveBeenCalledTimes(1);
  viewport.dispatchEvent(new Event('resize'));
  const pending = sequence;
  if (typeof cleanup === 'function') cleanup();
  expect(view.cancelAnimationFrame).toHaveBeenLastCalledWith(pending);
  expect(frames.size).toBe(0);
  const scheduled = view.requestAnimationFrame.mock.calls.length;
  viewport.dispatchEvent(new Event('resize'));
  expect(view.requestAnimationFrame).toHaveBeenCalledTimes(scheduled);
});
