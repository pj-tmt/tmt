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
  metadata: ReactNode;
  state: ReactNode;
  actions?: ReactNode;
}
export function BrowserListRow({
  title,
  metadata,
  state,
  actions,
  ...attributes
}: BrowserListRowProps) {
  return (
    <li {...attributes} className={c.listRow}>
      <div className={c.listTitle}>{title}</div>
      <div className={c.listMeta}>
        <div className={c.listUpdated}>{metadata}</div>
        <div className={c.listState}>{state}</div>
      </div>
      {actions !== undefined && <div className={c.listActions}>{actions}</div>}
    </li>
  );
}
