import { useEffect, useMemo } from 'react';
import { BrowserShell } from '../src/react.js';
import {
  shellCount,
  shellRail,
  type ShellAnswer,
  type ShellCommonControls,
  type ShellMenu,
  type ShellMenuItem,
  type ShellModule,
} from '../src/shell.js';
import { Button, Separator, UiTheme } from '../src/ui/index.js';
import { ListScreen } from './mock-list.js';
import { ManageDialog, menuControls, SettingsScreen } from './mock-menu.js';
import { ReaderScreen } from './mock-reader.js';
import { radiusLabel, setRadius, setTheme, usePreview, type Radius } from './preview.js';
import { follow, navigate, setQuery, useQuery } from './router.js';

/*
 * The Colab mockup (#5821): the side menu, the page list, the reader and Colab settings, built from the
 * design-system parts in palette B. The rail and the module menu are the shared shell; everything in
 * the content area is a `ui` part. Each screen lists the pieces that are stand-ins until a part exists.
 */

const base = '/mock/colab';
const colab: ShellModule = {
  extension: 'colab',
  rail: { label: 'Colab', icon: 'file-text', path: `${base}`, order: 100 },
  menu: [],
  settings: [],
  needsYou: 4,
};
const answer: ShellAnswer = {
  version: 1,
  modules: [
    colab,
    {
      extension: 'remote',
      rail: { label: 'Remote', icon: 'monitor-smartphone', path: '/mock/remote', order: 200 },
      menu: [],
      settings: [],
      needsYou: 1,
    },
    {
      extension: 'ops',
      rail: { label: 'Ops', icon: 'users', path: '/mock/ops', order: 300 },
      menu: [],
      settings: [],
      needsYou: 0,
    },
  ],
};
const common: ShellCommonControls = {
  openSearch: () => {},
  remote: { kind: 'confirmed', liveSessions: 2, href: base },
};

type Screen = 'menu' | 'list' | 'page' | 'settings';
const screens: { id: Screen; label: string; href: string }[] = [
  { id: 'menu', label: 'Side menu', href: `${base}/menu` },
  { id: 'list', label: 'Page list', href: base },
  { id: 'page', label: 'Page detail', href: `${base}/page` },
  { id: 'settings', label: 'Settings', href: `${base}/settings` },
];

interface Entry {
  id: string;
  label: string;
  href?: string;
  total?: number;
  needsYou?: number;
  unread?: boolean;
  children?: Entry[];
}
/** Colab's menu with #5826 proposal C: All pages, then the Shortcuts group with Manage. */
const shortcuts: Entry[] = [
  { id: 'spec', label: 'spec', total: 19, needsYou: 3 },
  { id: 'night', label: 'night', total: 1 },
  { id: 'pm', label: 'pm', total: 2 },
  { id: 'design', label: 'design', total: 3, needsYou: 1 },
  { id: 'blog', label: 'blog', total: 2 },
];

function buildMenu(current: string, withShortcuts: boolean, needs: boolean): ShellMenu {
  const item = (entry: Entry): ShellMenuItem => ({
    id: entry.id,
    label: entry.label,
    href: entry.href ?? `${base}#${entry.id}`,
    current: entry.id === current,
    total: entry.total,
    count: needs && entry.needsYou ? shellCount(entry.needsYou) : undefined,
  });
  return {
    title: 'Colab',
    groups: [
      { items: [item({ id: 'all', label: 'All pages', href: base, total: 99, needsYou: 16 })] },
      {
        label: 'Shortcuts',
        action: { label: 'Manage', icon: 'settings', onActivate: () => setQuery({ manage: '1' }) },
        items: withShortcuts ? shortcuts.map(item) : [],
      },
    ],
    settings: {
      id: 'settings',
      label: 'Colab settings',
      href: `${base}/settings`,
      current: current === 'settings',
    },
  };
}

/** The mock's global switches: screen, theme and radius, and the way back to the docs. */
export function GlobalSwitches({ screen }: { screen: Screen }) {
  const { theme, radius } = usePreview();
  return (
    <>
      <p className="mock-switch-label">Screen</p>
      <div className="mock-switch-row">
        {screens.map((entry) => (
          <Button
            key={entry.id}
            type="button"
            size="xs"
            variant={entry.id === screen ? 'secondary' : 'ghost'}
            aria-pressed={entry.id === screen}
            onClick={() => navigate(entry.href)}
          >
            {entry.label}
          </Button>
        ))}
      </div>
      <p className="mock-switch-label">Theme</p>
      <div className="mock-switch-row">
        {(['dark', 'light'] as const).map((value) => (
          <Button
            key={value}
            type="button"
            size="xs"
            variant={theme === value ? 'secondary' : 'ghost'}
            aria-pressed={theme === value}
            onClick={() => setTheme(value)}
          >
            {value === 'dark' ? 'Dark' : 'Light'}
          </Button>
        ))}
      </div>
      <p className="mock-switch-label">Radius</p>
      <div className="mock-switch-row">
        {(['2', '4', 'current'] as Radius[]).map((value) => (
          <Button
            key={value}
            type="button"
            size="xs"
            variant={radius === value ? 'secondary' : 'ghost'}
            aria-pressed={radius === value}
            onClick={() => setRadius(value)}
          >
            {radiusLabel[value]}
          </Button>
        ))}
      </div>
      <Separator />
      <a className="mock-switch-link" href="/docs" onClick={follow}>
        ← Design system docs
      </a>
    </>
  );
}

export function ColabMock({ path }: { path: string }) {
  const screen: Screen = path.startsWith(`${base}/page`)
    ? 'page'
    : path.startsWith(`${base}/menu`)
      ? 'menu'
      : path.startsWith(`${base}/settings`)
        ? 'settings'
        : 'list';
  const { theme } = usePreview();
  const query = useQuery();
  const withShortcuts = query.get('shortcuts') !== 'none';
  const needs = query.get('dots') !== 'off';
  const view = useMemo(() => {
    const place = { origin: location.origin, href: `${location.origin}${base}` };
    return {
      rail: shellRail(answer, place, colab),
      menu: buildMenu(
        screen === 'settings' ? 'settings' : screen === 'page' ? '' : 'all',
        withShortcuts,
        needs,
      ),
    };
  }, [screen, withShortcuts, needs]);
  // The side-menu screen opens the drawer on a phone, where the menu otherwise waits behind its button.
  useEffect(() => {
    if (screen !== 'menu' || !matchMedia('(max-width: 900px)').matches) return;
    document.querySelector<HTMLButtonElement>('.tmt-ui-shell-open')?.click();
  }, [screen]);
  const globals = <GlobalSwitches screen={screen} />;
  return (
    <UiTheme theme={theme}>
      <BrowserShell
        view={view}
        commonControls={common}
        select={(_, href) => {
          const target = new URL(href, location.origin);
          if (target.pathname.startsWith(base) && !target.hash) navigate(target.pathname);
          return true;
        }}
      >
        {screen === 'page' ? (
          <ReaderScreen globals={globals} />
        ) : screen === 'settings' ? (
          <SettingsScreen globals={globals} />
        ) : (
          <ListScreen globals={globals} controls={screen === 'menu' ? menuControls : undefined} />
        )}
      </BrowserShell>
      {query.get('manage') === '1' && <ManageDialog />}
    </UiTheme>
  );
}
