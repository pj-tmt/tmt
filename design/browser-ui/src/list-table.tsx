import { createElement, type ReactNode } from 'react';
import type { ListTableColumn, ListTableRowData } from './list-table-host.js';
export type { ListTableColumn, ListTableRowData } from './list-table-host.js';

export function BrowserListTable({
  label,
  className,
  columns,
  rows,
}: {
  label: string;
  className?: string;
  /** `header` replaces the label text in the column header (a select-all control, say). */
  columns: readonly (ListTableColumn & { header?: ReactNode })[];
  rows: readonly { id: string; cells: readonly ReactNode[]; data?: ListTableRowData }[];
}) {
  return (
    <div
      className={['tmt-ui-list-table', className].filter(Boolean).join(' ')}
      role="table"
      aria-label={label}
    >
      <div className="tmt-ui-list-columns" role="row">
        {columns.map((column) => (
          <span key={column.id} role="columnheader" data-hide-below={column.hideBelow}>
            {column.header ?? column.label}
          </span>
        ))}
      </div>
      <div role="rowgroup">
        {rows.map((row) => {
          if (row.cells.length !== columns.length)
            throw new Error('A cell is required for every column.');
          return (
            <div
              className="tmt-ui-list-table-row"
              role="row"
              key={row.id}
              {...Object.fromEntries(
                Object.entries(row.data ?? {}).map(([name, value]) => [`data-${name}`, value]),
              )}
            >
              {columns.map((column, index) => (
                <div
                  role="cell"
                  key={column.id}
                  data-column={column.id}
                  data-hide-below={column.hideBelow}
                >
                  {row.cells[index]}
                </div>
              ))}
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function BrowserPageSubheader({
  title,
  count,
  subtitle,
  actions,
  headingLevel = 2,
}: {
  title: string;
  count?: number;
  subtitle?: string;
  actions?: ReactNode;
  headingLevel?: 2 | 3 | 4 | 5 | 6;
}) {
  return (
    <header className="tmt-ui-subheader">
      <div className="tmt-ui-subheader-text">
        {createElement(
          `h${headingLevel}`,
          null,
          title,
          count !== undefined && (
            <>
              {' '}
              <span className="tmt-ui-shell-count">{count}</span>
            </>
          ),
        )}
        {subtitle !== undefined && <p>{subtitle}</p>}
      </div>
      {actions !== undefined && <div className="tmt-ui-subheader-actions">{actions}</div>}
    </header>
  );
}
