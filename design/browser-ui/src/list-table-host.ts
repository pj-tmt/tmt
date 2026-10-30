/** A presentational column: layout and visibility only, never data or interaction. */
export interface ListTableColumn {
  id: string;
  label: string;
  hideBelow?: 'phone';
}
/** Framework-free owner for the same table presentation used by the React entry. */
export function createListTable(
  columns: readonly ListTableColumn[],
  label: string,
  className?: string,
) {
  const root = document.createElement('div');
  root.className = ['tmt-ui-list-table', className].filter(Boolean).join(' ');
  root.setAttribute('role', 'table');
  root.setAttribute('aria-label', label);
  const header = document.createElement('div');
  header.className = 'tmt-ui-list-columns';
  header.setAttribute('role', 'row');
  for (const column of columns) {
    const cell = document.createElement('span');
    cell.setAttribute('role', 'columnheader');
    if (column.hideBelow) cell.dataset.hideBelow = column.hideBelow;
    cell.textContent = column.label;
    header.append(cell);
  }
  const body = document.createElement('div');
  body.setAttribute('role', 'rowgroup');
  root.append(header, body);
  return {
    root,
    body,
    row(cells: readonly Node[]) {
      if (cells.length !== columns.length) throw new Error('A cell is required for every column.');
      const row = document.createElement('div');
      row.className = 'tmt-ui-list-table-row';
      row.setAttribute('role', 'row');
      cells.forEach((content, index) => {
        const cell = document.createElement('div');
        cell.setAttribute('role', 'cell');
        if (columns[index]?.hideBelow) cell.dataset.hideBelow = columns[index]!.hideBelow;
        cell.append(content);
        row.append(cell);
      });
      return row;
    },
  };
}

/** Framework-free subheader. The caller owns its actions and the top-bar title. */
export function createPageSubheader(options: {
  title: string;
  count?: number;
  subtitle?: string;
  actions?: Node;
  headingLevel?: 2 | 3 | 4 | 5 | 6;
}): HTMLElement {
  const root = document.createElement('header');
  root.className = 'tmt-ui-subheader';
  const text = document.createElement('div');
  text.className = 'tmt-ui-subheader-text';
  const heading = document.createElement(`h${options.headingLevel ?? 2}`);
  heading.textContent = options.title;
  if (options.count !== undefined) {
    const count = document.createElement('span');
    count.className = 'tmt-ui-shell-count';
    count.textContent = String(options.count);
    heading.append(' ', count);
  }
  text.append(heading);
  if (options.subtitle !== undefined) {
    const subtitle = document.createElement('p');
    subtitle.textContent = options.subtitle;
    text.append(subtitle);
  }
  root.append(text);
  if (options.actions) {
    const actions = document.createElement('div');
    actions.className = 'tmt-ui-subheader-actions';
    actions.append(options.actions);
    root.append(actions);
  }
  return root;
}
