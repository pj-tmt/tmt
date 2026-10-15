import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it, vi } from 'vite-plus/test';
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
