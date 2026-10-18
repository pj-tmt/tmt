import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import { BrowserChip, BrowserChips } from '../src/react.js';

it('shares one chip primitive with full remove names and no host-value mutation', () => {
  const html = renderToStaticMarkup(
    <BrowserChip
      label="label:design"
      removeLabel="Remove label filter design"
      onRemove={() => {}}
    />,
  );
  expect(html).toContain('class="tmt-ui-chip"');
  expect(html).toContain('label:design');
  expect(html).toContain('aria-label="Remove label filter design"');
  expect(html).not.toContain('role="combobox"');
});
it('draws host content instead of the label while the label stays the fallback and name', () => {
  const chip = renderToStaticMarkup(
    <BrowserChip
      label="@x"
      content={<b data-mark>X</b>}
      removeLabel="Remove @x"
      onRemove={() => {}}
    />,
  );
  expect(chip).toContain('<b data-mark="true">X</b>');
  expect(chip).not.toContain('tmt-ui-chip-label">@x');
  expect(chip).toContain('aria-label="Remove @x"');
  const plain = renderToStaticMarkup(
    <BrowserChip label="@x" removeLabel="Remove @x" onRemove={() => {}} />,
  );
  expect(plain).toContain('tmt-ui-chip-label">@x</span>');
  const props = {
    label: 'Recipients',
    query: '',
    onQueryChange() {},
    onAdd() {},
    onRemove() {},
    addLabel: 'Add recipient',
    searchLabel: 'Search agents',
    emptyText: '',
    items: [{ id: 'x', label: '@x', content: <b data-mark>X</b>, removeLabel: 'Remove @x' }],
    options: [],
  };
  const list = renderToStaticMarkup(<BrowserChips {...props} />);
  expect(list).toContain('<b data-mark="true">X</b>');
});
it('exposes host empty/cap/disabled reasons while keeping values controlled', () => {
  const props = {
    label: 'Recipients',
    options: [],
    query: '',
    onQueryChange() {},
    onAdd() {},
    onRemove() {},
    addLabel: 'Add recipient',
    searchLabel: 'Search agents',
    emptyText: 'Stays on the page, no one is asked',
    maxItems: 1,
    limitText: 'At most 1',
  };
  expect(renderToStaticMarkup(<BrowserChips {...props} items={[]} />)).toContain(props.emptyText);
  const items = [{ id: 'x', label: '@x', removeLabel: 'Remove @x' }];
  const html = renderToStaticMarkup(<BrowserChips {...props} items={items} />);
  expect(html).toContain('At most 1');
  expect(html).toMatch(/class="tmt-ui-chips-add"[^>]*disabled=""/);
  expect(html).toContain('aria-label="Remove @x"');
  expect(items).toEqual([{ id: 'x', label: '@x', removeLabel: 'Remove @x' }]);
});
