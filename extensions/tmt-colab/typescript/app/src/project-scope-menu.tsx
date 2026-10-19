import { BrowserMenu } from '@tmt/browser-ui/react';
import { text } from './strings.js';
import type { ScopeRow } from './project-scope.js';
import './project-scope-menu.css';

const t = text.pageList;

/** The header's project crumb: the scope of the list, and where projects are created and managed. */
export function ProjectScopeMenu({
  rows,
  scope,
  onScope,
  onNew,
  onManage,
}: {
  rows: readonly ScopeRow[];
  /** The project the list is scoped to; `undefined` is All pages. */
  scope: string | undefined;
  onScope(name: string | undefined): void;
  /** Absent where this viewer cannot manage projects. */
  onNew?(): void;
  onManage?(): void;
}) {
  return (
    <BrowserMenu
      label={t.scopeLabel}
      valueLabel={scope ?? t.allPages}
      variant="crumb"
      items={[
        ...rows.map((row) => ({
          key: row.name === undefined ? 'all' : `project:${row.name}`,
          label: row.name ?? t.allPages,
          role: 'menuitemradio' as const,
          checked: row.name === scope,
          content: (
            <span className="scope-row">
              <span className="scope-name">{row.name ?? t.allPages}</span>
              <span className="scope-count">{row.count}</span>
              <span
                className="scope-needs"
                {...(row.waiting > 0
                  ? { role: 'img', 'aria-label': t.needsYouFilter(row.waiting) }
                  : { 'aria-hidden': true })}
              >
                {row.waiting > 0 ? `◆ ${row.waiting}` : ''}
              </span>
            </span>
          ),
        })),
        ...(onNew ? [{ key: 'new', label: `${t.newProject}…`, separated: true }] : []),
        ...(onManage ? [{ key: 'manage', label: `${t.manageProjects}…` }] : []),
      ]}
      onSelect={(key) => {
        if (key === 'new') onNew?.();
        else if (key === 'manage') onManage?.();
        else onScope(key === 'all' ? undefined : key.slice('project:'.length));
      }}
    />
  );
}
