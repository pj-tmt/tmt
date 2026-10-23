import type { HTMLAttributes, ReactNode } from 'react';
import { browserUiClasses as c } from './static.js';

export interface BrowserListProps {
  label: string;
  children: ReactNode;
  className?: string;
}
export function BrowserList({ label, children, className }: BrowserListProps) {
  return (
    <ul className={[c.list, className].filter(Boolean).join(' ')} aria-label={label}>
      {children}
    </ul>
  );
}

export interface BrowserListRowProps extends Omit<
  HTMLAttributes<HTMLLIElement>,
  'title' | 'children' | 'className' | 'style'
> {
  title: ReactNode;
  /** A line under the title; the title then wraps instead of truncating. */
  description?: ReactNode;
  /** The row's updated text; without it the state alone keeps the right edge. */
  metadata?: ReactNode;
  state: ReactNode;
  /** Colours the leading mark and the state word. */
  tone?: 'working' | 'waiting' | 'blocked' | 'review';
  actions?: ReactNode;
  /** Content that belongs to this row alone, such as the form that acts on it: under the name. */
  detail?: ReactNode;
  /** A control that belongs to the row before its title, such as a selection checkbox. */
  leading?: ReactNode;
}
export function BrowserListRow({
  title,
  description,
  metadata,
  state,
  tone,
  actions,
  detail,
  leading,
  ...attributes
}: BrowserListRowProps) {
  return (
    <li {...attributes} className={c.listRow} data-leading={leading !== undefined || undefined}>
      {leading !== undefined && (
        <div className={c.listLeading} data-tone={tone}>
          {leading}
        </div>
      )}
      <div className={c.listTitle}>
        {description === undefined ? (
          title
        ) : (
          <>
            <span className={c.listLabel}>{title}</span>
            <span className={c.listDescription}>{description}</span>
          </>
        )}
      </div>
      <div className={c.listMeta}>
        {metadata !== undefined && <div className={c.listUpdated}>{metadata}</div>}
        <div className={c.listState} data-tone={tone}>
          {state}
        </div>
      </div>
      {actions !== undefined && <div className={c.listActions}>{actions}</div>}
      {detail !== undefined && <div className={c.listDetail}>{detail}</div>}
    </li>
  );
}
