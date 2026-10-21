import type { ReactNode } from 'react';
import { browserUiClasses as c } from './static.js';

export interface BrowserTableProps {
  /** Names the table for assistive technology. */
  label: string;
  /** Column headings; the first column's cells are the row headings. */
  columns: readonly string[];
  rows: readonly (readonly ReactNode[])[];
  /** Right-align, tabulate and monospace every column after the first. */
  numeric?: boolean;
}
export function BrowserTable({ label, columns, rows, numeric }: BrowserTableProps) {
  return (
    <table className={c.table} aria-label={label} data-numeric={numeric || undefined}>
      <thead>
        <tr>
          {columns.map((column) => (
            <th key={column} scope="col">
              {column}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((row, index) => (
          <tr key={index}>
            <th scope="row">{row[0]}</th>
            {row.slice(1).map((value, column) => (
              <td key={column}>{value}</td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}
