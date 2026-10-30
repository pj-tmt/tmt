import { expect, it } from 'vite-plus/test';
import { renderToStaticMarkup } from 'react-dom/server';
import { BrowserListTable, BrowserPageSubheader } from '../src/list-table.js';

it('uses one shared grid for headers and cells and names columns semantically', () => {
  const columns = [
    { id: 'name', label: 'Name', track: '2fr' },
    { id: 'role', label: 'Role', track: '1fr', hideBelow: 'phone' as const },
  ];
  const html = renderToStaticMarkup(
    <BrowserListTable
      label="People"
      columns={columns}
      rows={[{ id: 'one', cells: ['Ben', 'Owner'] }]}
    />,
  );
  expect(html).toContain('--tmt-ui-list-tracks:2fr 1fr');
  expect(html).toContain('role="columnheader"');
  expect(html).toContain('data-hide-below="phone"');
  expect(html).toContain('Ben');
});

it('renders the subheader optional count, subtitle and actions without inventing them', () => {
  const basic = renderToStaticMarkup(<BrowserPageSubheader title="People" />);
  expect(basic).not.toContain('tmt-ui-shell-count');
  const full = renderToStaticMarkup(
    <BrowserPageSubheader
      title="People"
      count={4}
      subtitle="Four people"
      actions={<button>Invite</button>}
    />,
  );
  expect(full).toContain('tmt-ui-shell-count');
  expect(full).toContain('Four people');
  expect(full).toContain('Invite');
});
