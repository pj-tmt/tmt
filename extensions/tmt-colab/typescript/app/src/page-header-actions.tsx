import { RemoteMoreMenu, useRemoteHeaderSource } from './remote-header.js';
import { useContext, useSyncExternalStore } from 'react';
import { PageMenu as BrowserMenu } from './page-menu.js';
import type { BrowserMenuItem } from '@tmt/browser-ui/react';
import {
  Share2,
  Folder,
  CircleHelp,
  UserRound,
  Paperclip,
  Code2,
  Download,
  Bot,
  Settings,
  Info,
  Pin,
} from 'lucide-react';
import { getThemeChoice, setThemeChoice, subscribeTheme, type ThemeChoice } from './theme.js';
import { text } from './strings.js';
import { pinMenuItem, type PagePinState } from './pins.js';
import { SharedTopBar } from './shared-top-bar.js';
import { useMedia } from './use-media.js';

export type PageHeaderAction =
  | 'chat'
  | 'comments'
  | 'agents'
  | 'files'
  | 'source'
  | 'export'
  | 'manage'
  | 'about'
  | 'questions'
  | 'creator';

/** The More trigger's decorative icon; hosts retain each action's authority. */
function MoreIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
      <circle cx="5" cy="12" r="1" />
      <circle cx="12" cy="12" r="1" />
      <circle cx="19" cy="12" r="1" />
    </svg>
  );
}

/** Where the More trigger is (the last visible button of the page header's actions, outside any open
 * menu), for a menu opened from one of its items that belongs under it. */
export function moreButtonRect(): DOMRect | undefined {
  const rects = [...document.querySelectorAll('.page-header-actions button')]
    .filter((button) => !button.closest('dialog, [role="menu"]'))
    .map((button) => button.getBoundingClientRect())
    .filter((rect) => rect.width > 0);
  return rects[rects.length - 1];
}

/** The header holds the More menu; Comments and Chat are in the page's dock. */
export function PageHeaderActions({
  files,
  canExport,
  canManage,
  readOnly = false,
  canFiles,
  canProjects,
  pin,
  creatorLabel,
  access,
  onSelect,
}: {
  files: number;
  canExport: boolean;
  canManage: boolean;
  canFiles: boolean;
  /** The viewer can set the page's projects (the picker is beside the title). */
  canProjects: boolean;
  pin?: PagePinState;
  creatorLabel?: string;
  readOnly?: boolean;
  /** The page's sharing and connection words, the secondary text of the Share… item. */
  access: string;
  onSelect(action: PageHeaderAction | 'projects' | 'pin'): void;
}) {
  const choice = useSyncExternalStore(subscribeTheme, getThemeChoice);
  // The shared top bar owns Theme and the Remote link; the menu keeps neither.
  const sharedBar = useContext(SharedTopBar);
  const remoteAvailable = useRemoteHeaderSource() && !sharedBar;
  // The bar hides the page's chips and pin up to 900px; More offers the pin there instead.
  const compact = useMedia('(max-width: 900px)');
  const pinItem = pinMenuItem(pin, compact, canProjects, readOnly);
  const legacyItems: BrowserMenuItem[] = [
    {
      key: 'share',
      label: text.shareItem,
      content: (
        <span className="more-item">
          <span>{text.shareItem}</span>
          <span className="more-item-detail">{access}</span>
        </span>
      ),
      disabled: !canManage,
    },
    ...(canProjects ? [{ key: 'projects', label: text.projectsItem }] : []),
    ...(pinItem ? [pinItem] : []),
    { key: 'questions', label: text.metadata.questionsItem },
    { key: 'creator', label: text.metadata.creatorItem(creatorLabel) },
    ...(!readOnly ? [{ key: 'agents', label: text.agentStatus }] : []),
    // A page that cannot hold a file and holds none has nothing to open; a writer keeps the entry,
    // which is where files are added.
    ...(files > 0 || canFiles
      ? [{ key: 'files', label: `${text.files} (${files})`, disabled: !canFiles }]
      : []),
    { key: 'source', label: text.source },
    ...(!readOnly ? [{ key: 'export', label: text.export, disabled: !canExport }] : []),
    { key: 'manage', label: text.managePage, disabled: !canManage, separated: true },
    ...(sharedBar
      ? []
      : (['light', 'dark', 'system'] as const).map((theme) => ({
          key: theme,
          label: text.themeChoice[theme],
          checked: choice === theme,
        }))),
    { key: 'about', label: text.aboutPage },
  ];
  const groups: { heading: string; items: BrowserMenuItem[] }[] = [
    {
      heading: text.moreGroups.share,
      items: [
        {
          key: 'share',
          label: text.shareItem,
          detail: access,
          leadingIcon: <Share2 />,
          disabled: !canManage,
        },
      ],
    },
    {
      heading: text.moreGroups.organize,
      items: [
        ...(canProjects
          ? [{ key: 'projects', label: text.projectsItem, leadingIcon: <Folder /> }]
          : []),
        ...(pinItem ? [{ ...pinItem, leadingIcon: <Pin /> }] : []),
        { key: 'questions', label: text.metadata.questionsItem, leadingIcon: <CircleHelp /> },
        { key: 'creator', label: text.moreCreator, meta: creatorLabel, leadingIcon: <UserRound /> },
      ],
    },
    {
      heading: text.moreGroups.content,
      items: [
        ...(files > 0 || canFiles
          ? [
              {
                key: 'files',
                label: text.files,
                meta: String(files),
                leadingIcon: <Paperclip />,
                disabled: !canFiles,
              },
            ]
          : []),
        { key: 'source', label: text.source, leadingIcon: <Code2 /> },
        ...(!readOnly
          ? [{ key: 'export', label: text.export, leadingIcon: <Download />, disabled: !canExport }]
          : []),
      ],
    },
    {
      heading: text.moreGroups.page,
      items: [
        ...(!readOnly ? [{ key: 'agents', label: text.agentStatus, leadingIcon: <Bot /> }] : []),
        { key: 'manage', label: text.managePage, leadingIcon: <Settings />, disabled: !canManage },
        { key: 'about', label: text.aboutPage, leadingIcon: <Info /> },
      ],
    },
  ].filter((group) => group.items.length > 0);
  const groupedItems = groups.flatMap((group, groupIndex) =>
    group.items.map((item, index) =>
      index === 0 ? { ...item, heading: group.heading, separated: groupIndex > 0 } : item,
    ),
  );
  const more = {
    items: sharedBar ? groupedItems : legacyItems,
    onSelect: (key: string) => {
      if (key === 'share') onSelect('manage');
      else if (key === 'projects') onSelect('projects');
      else if (key === 'pin') onSelect('pin');
      else if (
        key === 'questions' ||
        key === 'creator' ||
        key === 'agents' ||
        key === 'manage' ||
        key === 'about' ||
        key === 'files' ||
        key === 'source' ||
        key === 'export'
      )
        onSelect(key);
      else setThemeChoice(key as ThemeChoice);
    },
  };
  return (
    <nav className="page-header-actions" aria-label={text.pageHeaderActions}>
      {remoteAvailable ? (
        <RemoteMoreMenu remoteAfter="share" {...more} />
      ) : (
        <BrowserMenu
          label={text.more}
          icon={<MoreIcon />}
          popupLayout={sharedBar ? 'grouped' : undefined}
          {...more}
        />
      )}
    </nav>
  );
}
