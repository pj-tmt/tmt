import { RemoteMoreMenu, useRemoteHeaderSource } from './remote-header.js';
import { BrowserIconAction } from '@tmt/browser-ui/react';
import { useSyncExternalStore } from 'react';
import { useMedia } from './use-media.js';
import { PageMenu as BrowserMenu } from './page-menu.js';
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

const compactQuery = '(max-width: 640px)';

/** Under 480px the header hides the access chip (CSS); its words lead the More menu instead. */
const phoneQuery = '(max-width: 479px)';

/** Page-local commands use the shared Menu presentation owner. */
export function PageHeaderActions({
  active,
  comments,
  files,
  unseen,
  chatUnread = false,
  canExport,
  canManage,
  canFiles,
  readOnly = false,
  access,
  onSelect,
}: {
  active: PageHeaderAction | null;
  comments: number;
  files: number;
  unseen: boolean;
  chatUnread?: boolean;
  canExport: boolean;
  canManage: boolean;
  canFiles: boolean;
  readOnly?: boolean;
  /** The access chip's words; shown as the More menu's first, non-interactive line on a phone. */
  access?: string;
  onSelect(action: PageHeaderAction): void;
}) {
  const compact = useMedia(compactQuery);
  const phone = useMedia(phoneQuery);
  const remoteAvailable = useRemoteHeaderSource();
  const MoreMenu = compact && remoteAvailable ? RemoteMoreMenu : BrowserMenu;
  const choice = useSyncExternalStore(subscribeTheme, getThemeChoice);
  return (
    <nav className="page-header-actions" aria-label={text.pageHeaderActions}>
      <BrowserMenu
        label={`${text.discussion} (${comments})${unseen ? ` · ${text.threadUnseen}` : ''}`}
        icon={<PageHeaderIcon name="discussion" count={comments} unseen={unseen} />}
        items={[
          { key: 'chat', label: chatUnread ? `${text.chat} · ${text.chatUnread}` : text.chat },
          {
            key: 'comments',
            label: `${text.comments} ${comments}${unseen ? ` · ${text.threadUnseen}` : ''}`,
          },
        ]}
        onSelect={(key) => onSelect(key as 'chat' | 'comments')}
      />
      {!compact && (
        <>
          {!readOnly && (
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
          )}
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
          <BrowserMenu
            label={text.sourceAndExport}
            icon={<PageHeaderIcon name="source" />}
            items={[
              { key: 'source', label: text.source },
              ...(!readOnly ? [{ key: 'export', label: text.export, disabled: !canExport }] : []),
            ]}
            onSelect={(key) => onSelect(key as 'source' | 'export')}
          />
        </>
      )}
      <MoreMenu
        label={text.more}
        icon={<PageHeaderIcon name="more" />}
        description={phone ? access : undefined}
        items={[
          ...(compact
            ? [
                ...(!readOnly ? [{ key: 'agents', label: text.agentStatus }] : []),
                { key: 'files', label: `${text.files} (${files})`, disabled: !canFiles },
                { key: 'source', label: text.source },
                ...(!readOnly ? [{ key: 'export', label: text.export, disabled: !canExport }] : []),
              ]
            : []),
          { key: 'manage', label: text.managePage, disabled: !canManage },
          ...(['light', 'dark', 'system'] as const).map((theme) => ({
            key: theme,
            label: text.themeChoice[theme],
            checked: choice === theme,
          })),
          { key: 'about', label: text.aboutPage },
        ]}
        onSelect={(key) => {
          if (
            key === 'manage' ||
            key === 'about' ||
            key === 'agents' ||
            key === 'files' ||
            key === 'source' ||
            key === 'export'
          )
            onSelect(key);
          else setThemeChoice(key as ThemeChoice);
        }}
      />
    </nav>
  );
}
