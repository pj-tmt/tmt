import { BrowserIconAction } from '@tmt/browser-ui/react';
import { useSyncExternalStore } from 'react';
import { ActionMenu } from './components/action-menu.js';
import { getThemeChoice, setThemeChoice, subscribeTheme, type ThemeChoice } from './theme.js';
import { text } from './strings.js';

export type PageHeaderAction =
  | 'chat'
  | 'comments'
  | 'agents'
  | 'files'
  | 'source'
  | 'export'
  | 'manage'
  | 'about';
type HeaderIcon = 'discussion' | 'agents' | 'files' | 'source' | 'more';

/** One set of decorative page-header icons; hosts retain each action's authority. */
function PageHeaderIcon({
  name,
  count,
  unseen,
}: {
  name: HeaderIcon;
  count?: number;
  unseen?: boolean;
}) {
  return (
    <>
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        aria-hidden="true"
      >
        {name === 'discussion' ? (
          <path d="M4 4h16v12H9l-5 4V4Z" />
        ) : name === 'agents' ? (
          <>
            <path d="M7 20v-4l5-3 5 3v4M4 16v-3l3-2M20 16v-3l-3-2" />
            <circle cx="12" cy="7" r="3" />
            <path d="M5 5a3 3 0 0 0 0 6M19 5a3 3 0 0 1 0 6" />
          </>
        ) : name === 'files' ? (
          <path d="M5 3h9l5 5v13H5V3Zm9 0v5h5M8 12h8M8 16h8" />
        ) : name === 'source' ? (
          <path d="m7 6-5 6 5 6m10-12 5 6-5 6M14 3l-4 18" />
        ) : (
          <>
            <circle cx="5" cy="12" r="1" />
            <circle cx="12" cy="12" r="1" />
            <circle cx="19" cy="12" r="1" />
          </>
        )}
      </svg>
      {count !== undefined && <span className="page-header-count">{count}</span>}
      {unseen && <span className="page-header-unseen">•</span>}
    </>
  );
}

/** Page-local menus compose shared icon/tooltip presentation and the existing action-menu owner. */
export function PageHeaderActions({
  active,
  comments,
  files,
  unseen,
  canExport,
  canManage,
  canFiles,
  onSelect,
}: {
  active: PageHeaderAction | null;
  comments: number;
  files: number;
  unseen: boolean;
  canExport: boolean;
  canManage: boolean;
  canFiles: boolean;
  onSelect(action: PageHeaderAction): void;
}) {
  const choice = useSyncExternalStore(subscribeTheme, getThemeChoice);
  return (
    <nav className="page-header-actions" aria-label="Page actions">
      <ActionMenu
        label={`Discussion (${comments})${unseen ? ` · ${text.threadUnseen}` : ''}`}
        icon={<PageHeaderIcon name="discussion" count={comments} unseen={unseen} />}
        items={[
          { key: 'chat', label: 'Chat' },
          {
            key: 'comments',
            label: `${text.comments} ${comments}${unseen ? ` · ${text.threadUnseen}` : ''}`,
          },
        ]}
        onSelect={(key) => onSelect(key as 'chat' | 'comments')}
      />
      <BrowserIconAction
        type="button"
        variant="text"
        label={text.agentStatus}
        icon={<PageHeaderIcon name="agents" />}
        hasPopup="dialog"
        expanded={active === 'agents'}
        onActivate={(event) => {
          if (event.isTrusted) onSelect('agents');
        }}
      />
      <BrowserIconAction
        type="button"
        variant="text"
        label={`${text.files} (${files})`}
        icon={<PageHeaderIcon name="files" count={files} />}
        hasPopup="dialog"
        expanded={active === 'files'}
        disabled={!canFiles}
        onActivate={(event) => {
          if (event.isTrusted) onSelect('files');
        }}
      />
      <ActionMenu
        label="Source and export"
        icon={<PageHeaderIcon name="source" />}
        items={[
          { key: 'source', label: text.source },
          { key: 'export', label: text.export, disabled: !canExport },
        ]}
        onSelect={(key) => onSelect(key as 'source' | 'export')}
      />
      <ActionMenu
        label="More"
        icon={<PageHeaderIcon name="more" />}
        items={[
          { key: 'manage', label: 'Manage page', disabled: !canManage },
          ...(['light', 'dark', 'system'] as const).map((theme) => ({
            key: theme,
            label: `Theme: ${theme[0].toUpperCase()}${theme.slice(1)}`,
            checked: choice === theme,
          })),
          { key: 'about', label: 'About this page' },
        ]}
        onSelect={(key) => {
          if (key === 'manage' || key === 'about') onSelect(key);
          else setThemeChoice(key as ThemeChoice);
        }}
      />
    </nav>
  );
}
