import { createElement, type CSSProperties, type ReactNode } from 'react';
import { listTableTracks, type ListTableColumn } from './list-table-host.js';
export type { ListTableColumn } from './list-table-host.js';

export function BrowserListTable({
  label,
  columns,
  rows,
}: {
  label: string;
  columns: readonly ListTableColumn[];
  rows: readonly { id: string; cells: readonly ReactNode[] }[];
}) {
  return (
    <div
      className="tmt-ui-list-table"
      role="table"
      aria-label={label}
      style={{ '--tmt-ui-list-tracks': listTableTracks(columns) } as CSSProperties}
    >
      <div className="tmt-ui-list-columns" role="row">
        {columns.map((column) => (
          <span key={column.id} role="columnheader" data-hide-below={column.hideBelow}>
            {column.label}
          </span>
        ))}
      </div>
      <div role="rowgroup">
        {rows.map((row) => {
          if (row.cells.length !== columns.length)
            throw new Error('A cell is required for every column.');
          return (
            <div className="tmt-ui-list-table-row" role="row" key={row.id}>
              {columns.map((column, index) => (
                <div role="cell" key={column.id} data-hide-below={column.hideBelow}>
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
