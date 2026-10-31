import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { BrowserShell, BrowserTopBar, useTopBarReady } from '../src/react.js';
import {
  shellCount,
  shellRail,
  type ShellAnswer,
  type ShellCommonControls,
  type ShellMenu,
  type ShellMenuItem,
  type ShellModule,
} from '../src/shell.js';
import {
  Alert,
  AlertDescription,
  AlertTitle,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
  Input,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Separator,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  UiTheme,
} from '../src/ui/index.js';
import { ReaderScreen } from './mock-reader.js';
import { PageRow } from './patterns.js';
import { radiusLabel, setRadius, setTheme, usePreview, type Radius } from './preview.js';
import { follow, navigate } from './router.js';

/*
 * The Colab mockup (#5821): three screens built only from the design-system parts, in palette B. The
 * rail and the module menu are the shared shell (#5805's one-level children); everything in the content
 * area is a `ui` part. Each screen lists the pieces that are stand-ins until a real part exists.
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

type Screen = 'menu' | 'list' | 'page';
const screens: { id: Screen; label: string; href: string }[] = [
  { id: 'menu', label: 'Side menu', href: `${base}/menu` },
  { id: 'list', label: 'Page list', href: base },
  { id: 'page', label: 'Page detail', href: `${base}/page` },
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
const menuGroups: { label?: string; add?: string; entries: Entry[] }[] = [
  {
    entries: [
      {
        id: 'pages',
        label: 'Pages',
        href: base,
        total: 38,
        needsYou: 4,
        children: [
          { id: 'needs-you', label: 'Needs you', needsYou: 4 },
          { id: 'unread', label: 'Unread', total: 6, unread: true },
          { id: 'mine', label: 'Mine', total: 11 },
          { id: 'archived', label: 'Archived' },
        ],
      },
    ],
  },
  {
    label: 'Projects',
    add: 'New project',
    entries: [
      {
        id: 'release',
        label: 'Release 0.9',
        total: 7,
        needsYou: 2,
        children: [
          { id: 'release-plan', label: 'Release plan', unread: true },
          { id: 'rollout', label: 'Rollout checklist' },
          { id: 'notes', label: 'Release notes draft' },
        ],
      },
      { id: 'remote-launch', label: 'Remote launch', total: 5 },
      { id: 'night-shift', label: 'Night shift', total: 9, unread: true },
    ],
  },
  {
    label: 'Design',
    entries: [
      {
        id: 'design-system',
        label: 'Design system',
        total: 4,
        children: [
          { id: 'theme-b', label: 'Theme B review', href: `${base}/page`, unread: true },
          { id: 'tokens', label: 'Token tiers' },
          { id: 'components', label: 'Component audit' },
        ],
      },
      { id: 'brand', label: 'Brand sheet', total: 2 },
    ],
  },
];

function buildMenu(
  current: string,
  expanded: ReadonlySet<string>,
  toggle: (id: string) => void,
): ShellMenu {
  const item = (entry: Entry): ShellMenuItem => ({
    id: entry.id,
    label: entry.label,
    href: entry.href ?? `${base}#${entry.id}`,
    current: entry.id === current,
    total: entry.total,
    count: entry.needsYou ? shellCount(entry.needsYou) : undefined,
    emphasis: entry.unread ? 'unread' : undefined,
    ...(entry.children
      ? {
          children: entry.children.map(item),
          expanded: expanded.has(entry.id),
          toggle: () => toggle(entry.id),
        }
      : {}),
  });
  return {
    title: 'Colab',
    groups: menuGroups.map((group) => ({
      label: group.label,
      items: group.entries.map(item),
      add: group.add ? { label: group.add, onActivate: () => {} } : undefined,
    })),
    settings: { id: 'settings', label: 'Colab settings', href: `${base}#settings`, current: false },
  };
}

/** The top bar's breadcrumb: the trail in the quiet colour, the current page in the text colour. */
function Bar({ trail, current, actions }: { trail: string; current: string; actions: ReactNode }) {
  return useTopBarReady() ? (
    <BrowserTopBar
      title={
        <h1>
          <span className="tmt-ui-topbar-title-prefix">{trail}</span>
          <span className="tmt-ui-topbar-title-current">{current}</span>
        </h1>
      }
      actions={actions}
    />
  ) : null;
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

/** The mock's own switches, kept off the screens: a floating popover with screens, theme and radius. */
function MockSwitch({ screen }: { screen: Screen }) {
  return (
    <div className="mock-switch">
      <Popover>
        <PopoverTrigger asChild>
          <Button type="button" variant="outline" size="sm">
            Mock · {screens.find((entry) => entry.id === screen)!.label}
          </Button>
        </PopoverTrigger>
        <PopoverContent align="end" className="mock-switch-panel">
          <GlobalSwitches screen={screen} />
        </PopoverContent>
      </Popover>
    </div>
  );
}

/** What on this screen is not yet a real part. */
function StandIns({ items }: { items: string[] }) {
  return (
    <Alert className="mock-standins">
      <AlertTitle>Mock notes</AlertTitle>
      <AlertDescription>
        <ul>
          {items.map((item) => (
            <li key={item}>{item}</li>
          ))}
        </ul>
      </AlertDescription>
    </Alert>
  );
}

export function ColabMock({ path }: { path: string }) {
  const screen: Screen = path.startsWith(`${base}/page`)
    ? 'page'
    : path.startsWith(`${base}/menu`)
      ? 'menu'
      : 'list';
  const { theme } = usePreview();
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(
    () => new Set(['pages', 'release', 'design-system']),
  );
  const toggle = (id: string) =>
    setExpanded((now) => {
      const next = new Set(now);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const view = useMemo(() => {
    const place = { origin: location.origin, href: `${location.origin}${base}` };
    return {
      rail: shellRail(answer, place, colab),
      menu: buildMenu(screen === 'page' ? 'theme-b' : 'pages', expanded, toggle),
    };
  }, [screen, expanded]);
  // The side-menu screen opens the drawer on a phone, where the menu otherwise waits behind its button.
  useEffect(() => {
    if (screen !== 'menu' || !matchMedia('(max-width: 900px)').matches) return;
    document.querySelector<HTMLButtonElement>('.tmt-ui-shell-open')?.click();
  }, [screen]);
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
          <ReaderScreen globals={<GlobalSwitches screen={screen} />} />
        ) : screen === 'menu' ? (
          <MenuScreen />
        ) : (
          <PageList />
        )}
      </BrowserShell>
      {screen !== 'page' && <MockSwitch screen={screen} />}
    </UiTheme>
  );
}

function MenuScreen() {
  return (
    <>
      <Bar trail="Colab / " current="Home" actions={<Button type="button">New page</Button>} />
      <div className="mock-page">
        <section className="mock-section">
          <h2 className="mock-h2">Needs you</h2>
          <div className="mock-cards">
            {[
              ['Theme B review', 'Design system', 'Answer the radius question', 'waiting'],
              ['Release plan', 'Release 0.9', '2 open comments on the rollout order', 'waiting'],
              ['CI flake triage', 'Release 0.9', 'Blocked on the macOS runner image', 'blocked'],
            ].map(([title, project, why, state]) => (
              <Card key={title} className="mock-card">
                <CardHeader>
                  <CardTitle>{title}</CardTitle>
                  <CardDescription>{project}</CardDescription>
                </CardHeader>
                <CardContent>
                  <p className="mock-state" data-tone={state}>
                    {state === 'blocked' ? '✕' : '◆'} {why}
                  </p>
                </CardContent>
                <CardFooter>
                  <Button asChild variant="outline" size="sm">
                    <a href={`${base}/page`} onClick={follow}>
                      Open
                    </a>
                  </Button>
                </CardFooter>
              </Card>
            ))}
          </div>
        </section>
        <StandIns
          items={[
            'The rail and the module menu are the real shared shell; Pages, Release 0.9 and Design system open a nested level.',
            'Home cards are Card parts; a real Colab home is not designed yet.',
            'Search and the Remote chip in the bar are the shell’s own common controls, not wired here.',
          ]}
        />
      </div>
    </>
  );
}

const rows: {
  title: string;
  tags: string[];
  updated: string;
  unread?: boolean;
  state?: 'waiting' | 'blocked';
}[] = [
  {
    title: 'Theme B review',
    tags: ['design', 'spec'],
    updated: '2 min ago',
    unread: true,
    state: 'waiting',
  },
  { title: 'Release plan', tags: ['release-0.9'], updated: '18 min ago', unread: true },
  {
    title: 'Rollout checklist',
    tags: ['release-0.9', 'ops'],
    updated: '1 h ago',
    state: 'waiting',
  },
  { title: 'CI flake triage', tags: ['infra'], updated: '3 h ago', state: 'blocked' },
  { title: 'Remote pairing walkthrough', tags: ['remote-launch'], updated: 'Yesterday' },
  {
    title: 'Night shift report, 30 Oct',
    tags: ['night-shift'],
    updated: 'Yesterday',
    unread: true,
  },
  { title: 'Token tiers', tags: ['design'], updated: 'Mon' },
  { title: 'Brand sheet', tags: ['brand'], updated: '27 Oct' },
];
const projects = ['All projects', 'release-0.9', 'design', 'remote-launch', 'night-shift', 'infra'];

function PageList() {
  const [project, setProject] = useState('All projects');
  const [query, setQuery] = useState('');
  const shown = (tab: string) =>
    rows.filter(
      (row) =>
        (project === 'All projects' || row.tags.includes(project)) &&
        row.title.toLowerCase().includes(query.toLowerCase()) &&
        (tab === 'all' || (tab === 'needs' ? row.state : tab === 'unread' ? row.unread : false)),
    );
  return (
    <>
      <Bar trail="Colab / " current="Pages" actions={<Button type="button">New page</Button>} />
      <div className="mock-page">
        <Tabs defaultValue="all">
          <div className="mock-list-head">
            <TabsList variant="line">
              <TabsTrigger value="all">All 38</TabsTrigger>
              <TabsTrigger value="needs">Needs you 4</TabsTrigger>
              <TabsTrigger value="unread">Unread 6</TabsTrigger>
              <TabsTrigger value="archived">Archived</TabsTrigger>
            </TabsList>
          </div>
          <div className="mock-filters" role="group" aria-label="Filters">
            <Input
              className="mock-search"
              placeholder="Filter pages"
              aria-label="Filter pages"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
            <div className="mock-projects" role="group" aria-label="Project">
              {projects.map((name) => (
                <Button
                  key={name}
                  type="button"
                  size="xs"
                  variant={project === name ? 'secondary' : 'ghost'}
                  aria-pressed={project === name}
                  onClick={() => setProject(name)}
                >
                  {name}
                </Button>
              ))}
            </div>
            <Select defaultValue="updated">
              <SelectTrigger aria-label="Sort">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="updated">Last updated</SelectItem>
                <SelectItem value="title">Title</SelectItem>
                <SelectItem value="project">Project</SelectItem>
              </SelectContent>
            </Select>
          </div>
          {['all', 'needs', 'unread', 'archived'].map((tab) => (
            <TabsContent key={tab} value={tab}>
              <div className="mock-rows">
                {shown(tab).map((row) => (
                  <PageRow key={row.title} {...row} />
                ))}
                {shown(tab).length === 0 && <p className="mock-quiet">No pages match.</p>}
              </div>
            </TabsContent>
          ))}
        </Tabs>
        <StandIns
          items={[
            'Project filters are ghost/secondary Buttons with aria-pressed: there is no ToggleGroup part yet.',
            'Rows are the List row pattern (an anchor with Badge tags); a list-row part is not extracted yet.',
            'Tab counts are plain text in the trigger; a count slot for Tabs does not exist yet.',
          ]}
        />
      </div>
    </>
  );
}
