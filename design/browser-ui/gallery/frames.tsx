import { CompactControlsFrame } from './compact-controls-frame.js';
import { ChartsSection } from './charts.js';
import { MenuHostFrame } from './menu-host-frame.js';
import { RailFrame } from './rail-frame.js';
import { TopBarFrame } from './topbar-frame.js';
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Search, Settings } from 'lucide-react';
import {
  BrowserAction,
  BrowserDialog,
  BrowserDrawer,
  BrowserPanel,
  BrowserField,
  BrowserHeader,
  BrowserIconAction,
  BrowserList,
  BrowserListRow,
  BrowserMenu,
  BrowserNotifications,
  BrowserShell,
  HostedShell,
  useShellBar,
  useShellTools,
  type BrowserNotification,
} from '../src/react.js';
import {
  describedMenu,
  shellCount,
  shellRail,
  type ShellAnswer,
  type ShellView,
} from '../src/shell.js';
import { startShellHost, type ShellSlot } from '../src/shell-host.js';
import { FramePair, Section, noop } from './kit.js';
import { EditorialSection } from './editorial.js';
import { ThemeBFrame } from './theme-b.js';
import { menuItems, NoticeSection } from './components.js';
import { UiOverlayFrame } from './ui-overlays.js';
import { createIcon } from '../src/glyph.js';

/*
 * Frames are real pages of this gallery (`?frame=<name>&theme=<theme>`) shown in iframes, for the
 * shell's viewport layout and the top-layer overlays, which cannot share one page side by side.
 */

const origin = location.origin;
const answer: ShellAnswer = {
  version: 1,
  modules: [
    {
      extension: 'colab',
      rail: { label: 'Colab', icon: 'file-text', path: `${origin}/colab/`, order: 100 },
      menu: [
        {
          id: 'pages',
          label: 'Pages',
          path: `${origin}/colab/pages`,
          count: { value: 12, kind: 'plain' },
        },
        {
          id: 'needs-you',
          label: 'Needs you',
          path: `${origin}/colab/needs-you`,
          count: { value: 3, kind: 'needs-you' },
        },
        {
          id: 'release',
          label: 'Release planning',
          path: `${origin}/colab/p/release`,
          group: 'Projects',
        },
        { id: 'brand', label: 'Brand', path: `${origin}/colab/p/brand`, group: 'Projects' },
      ],
      settings: [
        { id: 'colab-settings', label: 'Colab settings', path: `${origin}/colab/settings` },
      ],
      needsYou: 3,
    },
    {
      extension: 'remote',
      rail: { label: 'Remote', icon: 'monitor-smartphone', path: `${origin}/remote/`, order: 200 },
      menu: [],
      settings: [],
      needsYou: 1,
    },
    {
      extension: 'ops',
      rail: { label: 'Ops', icon: 'users', path: `${origin}/ops/`, order: 300 },
      menu: [],
      settings: [],
      needsYou: 0,
    },
  ],
};

function ShellHeader() {
  const bar = useShellBar();
  const tools = useShellTools();
  return bar && tools ? (
    <>
      {createPortal(
        <BrowserHeader
          productLabel="Colab"
          title="Pages"
          inShell
          actions={
            <BrowserIconAction
              type="button"
              variant="text"
              label="Settings"
              icon={<Settings />}
              onActivate={noop}
            />
          }
        />,
        bar,
      )}
      {createPortal(
        <BrowserIconAction
          type="button"
          variant="text"
          label="Search pages"
          icon={
            <>
              <Search />
              <kbd>⌘K</kbd>
            </>
          }
          onActivate={noop}
        />,
        tools.search,
      )}
      {createPortal(
        <BrowserAction type="button" variant="text" label="Remote · 2" onActivate={noop} />,
        tools.chip,
      )}
    </>
  ) : null;
}

function ShellFrame({ params }: { params: URLSearchParams }) {
  const [projects, setProjects] = useState(params.get('projects') === '20' ? 20 : 3);
  const [mounted, setMounted] = useState(true);
  const scrollFixture = params.has('projects');
  const view = useMemo<ShellView>(() => {
    const path = params.get('page') === 'project' ? '/colab/p/release' : '/colab/pages';
    const place = { origin, href: `${origin}${path}` };
    const original = answer.modules[0]!;
    const colab = scrollFixture
      ? {
          ...original,
          menu: [
            original.menu[0]!,
            ...Array.from({ length: projects }, (_, index) => ({
              id: `project-${index}`,
              label: `Project ${index + 1}`,
              path: `${origin}/colab/p/${index}`,
              group: 'Projects',
            })),
          ],
        }
      : original;
    const menu = params.get('menu') === 'none' ? undefined : describedMenu(colab, place);
    if (menu) menu.groups[0]!.items[0]!.total = 12;
    if (menu) menu.groups[0]!.items[0]!.count = shellCount(2);
    // `counts` pins the needs-you range the rail must carry: none (Ops 0), one digit (Colab 3), two (Remote 12).
    const shown = params.has('counts')
      ? {
          ...answer,
          modules: answer.modules.map((module) =>
            module.extension === 'remote' ? { ...module, needsYou: 12 } : module,
          ),
        }
      : answer;
    return { rail: shellRail(shown, place, colab), menu };
  }, [params, projects, scrollFixture]);
  useEffect(() => {
    const shell = document.querySelector<HTMLElement>('.tmt-ui-shell')!;
    if (params.get('drawer') === 'open')
      shell.querySelector<HTMLButtonElement>('.tmt-ui-shell-open')!.click();
  }, [params]);
  if (!mounted) return <p>Shell disposed</p>;
  return (
    <BrowserShell view={view} select={() => true}>
      <ShellHeader />
      <div className="tmt-ui-page" style={scrollFixture ? { minHeight: 1600 } : undefined}>
        {scrollFixture && (
          <>
            <button type="button" onClick={() => setProjects(projects === 20 ? 3 : 20)}>
              Use {projects === 20 ? 3 : 20} projects
            </button>
            <button type="button" onClick={() => setMounted(false)}>
              Unmount shell
            </button>
          </>
        )}
        <header className="tmt-ui-page-head">
          <h2 className="tmt-ui-page-title">Your pages, in one place.</h2>
          <p className="tmt-ui-page-subtitle">A shared space for your team’s work.</p>
        </header>
        <section className="tmt-ui-card">
          <h3 className="tmt-ui-card-title">Pages</h3>
          <BrowserList label="Pages">
            <BrowserListRow
              title={<a href="#pages">Release plan for the remote channel</a>}
              metadata="ux-4 · 2 minutes ago"
              state={<span>◆ needs you</span>}
            />
            <BrowserListRow
              title={<a href="#pages">Brand sheet</a>}
              metadata="tmt-ux-lead · yesterday"
              state={<span>✓ answered</span>}
            />
          </BrowserList>
        </section>
      </div>
    </BrowserShell>
  );
}

/** The one TMT app: `startShellHost` owns the rail; a React module renders into its slot
 * through `HostedShell`, which sends the module menu to the host. */
function HostedFrame() {
  const root = useRef<HTMLDivElement>(null);
  const [slot, setSlot] = useState<ShellSlot>();
  useLayoutEffect(() => {
    // The frame is a page of this gallery (`/design/?frame=hosted`), so its one route is that page:
    // a route the host does not know would reload the frame and then rewrite its address.
    const host = startShellHost(root.current!, [
      {
        id: 'colab',
        paths: [window.location.pathname],
        rail: 'colab',
        load: async () => ({ mount: setSlot }),
      },
    ]);
    return () => host.dispose();
  }, []);
  const menu = useMemo(() => describedMenu(answer.modules[0]!, { origin, href: `${origin}/` }), []);
  return (
    <div ref={root} className="tmt-ui-shell">
      <div className="tmt-ui-shell-content">
        <div className="tmt-ui-shell-bar" />
        <div className="tmt-ui-shell-main" />
      </div>
      {slot &&
        createPortal(
          <HostedShell slot={slot} menu={menu} select={() => true}>
            <ShellHeader />
            <div className="gallery-frame-main">
              <p>A route module mounted by the host; the rail is the host's.</p>
            </div>
          </HostedShell>,
          slot.main,
        )}
    </div>
  );
}

function DialogFrame({ params }: { params: URLSearchParams }) {
  const [open, setOpen] = useState(params.get('open') !== 'false');
  const confirm = params.get('presentation') === 'confirm';
  const deleting = confirm && params.get('intent') === 'delete';
  return (
    <>
      <BrowserAction
        type="button"
        variant="text"
        label="Open dialog"
        onActivate={() => setOpen(true)}
      />
      <BrowserDialog
        open={open}
        title={confirm ? (deleting ? 'Delete page?' : 'Archive page?') : 'Move to project'}
        size={params.get('size') === 'wide' ? 'wide' : 'regular'}
        presentation={confirm ? 'confirm' : 'standard'}
        onDismiss={() => setOpen(false)}
        actions={
          <>
            <BrowserAction
              type="button"
              variant={confirm ? 'secondary' : 'text'}
              label="Cancel"
              onActivate={() => setOpen(false)}
            />
            <BrowserAction
              type="button"
              variant={deleting ? 'destructive' : 'primary'}
              label={confirm ? (deleting ? 'Delete page' : 'Archive page') : 'Move'}
              onActivate={() => setOpen(false)}
            />
          </>
        }
      >
        {confirm ? (
          <p>
            {deleting
              ? 'This permanently removes the page. Copies already made cannot be taken back.'
              : 'An archived page stays readable, but nobody can edit it. You can restore it from Archived.'}
          </p>
        ) : (
          <>
            <BrowserField
              controlId="dialog-project"
              label="Project"
              renderControl={(props) => <input {...props} defaultValue="Release planning" />}
            />
            <BrowserField
              controlId="dialog-note"
              label="Note"
              description="Shown to the project's members."
              descriptionId="dialog-note-description"
              renderControl={(props) => <input {...props} placeholder="Optional" />}
            />
          </>
        )}
      </BrowserDialog>
    </>
  );
}

function DrawerFrame({ params }: { params: URLSearchParams }) {
  const [open, setOpen] = useState(params.get('open') !== 'false');
  return (
    <>
      <BrowserAction
        type="button"
        variant="text"
        label="Open drawer"
        onActivate={() => setOpen(true)}
      />
      <BrowserDrawer
        open={open}
        title="Page settings"
        caption="Drafts stay here"
        panel="gallery"
        width={params.get('width') === 'wide' ? 'wide' : 'regular'}
        onDismiss={() => setOpen(false)}
        actions={
          <>
            <BrowserAction
              type="button"
              variant="text"
              label="Cancel"
              onActivate={() => setOpen(false)}
            />
            <BrowserAction
              type="button"
              variant="primary"
              label="Save"
              onActivate={() => setOpen(false)}
            />
          </>
        }
      >
        <BrowserField
          controlId="drawer-title"
          label="Title"
          renderControl={(props) => <input {...props} defaultValue="Release plan" />}
        />
        <BrowserMenu label="Layout" valueLabel="Wide" items={menuItems} onSelect={noop} />
      </BrowserDrawer>
    </>
  );
}

/** Source presentation only: Save records one local activation and remains busy. */
function SourceFrame({ params }: { params: URLSearchParams }) {
  const [open, setOpen] = useState(params.get('open') !== 'false');
  const [busy, setBusy] = useState(params.get('busy') === 'true');
  const [activations, setActivations] = useState(0);
  const save = (
    <BrowserAction
      type="button"
      variant="primary"
      label="Save"
      busy={busy}
      onActivate={() => {
        setActivations((count) => count + 1);
        setBusy(true);
      }}
    />
  );
  return (
    <div data-source-activations={activations}>
      <BrowserAction
        type="button"
        variant="text"
        label="Open source"
        onActivate={() => setOpen(true)}
      />
      <BrowserDrawer
        open={open}
        title="Source"
        width="wide"
        headerActions={params.has('legacy') ? undefined : save}
        actions={params.has('legacy') ? save : undefined}
        onDismiss={() => setOpen(false)}
      >
        <p>
          <span id="gallery-source-label">Page source</span>. Edit this sample HTML. This gallery
          never saves a page.
        </p>
        <textarea
          className="tmt-ui-field-control gallery-source-editor"
          aria-labelledby="gallery-source-label"
          spellCheck={false}
          defaultValue={
            '<section>\n  <h1>Navigation proposal</h1>\n  <p>One sidebar, less friction.</p>\n</section>'
          }
        />
      </BrowserDrawer>
    </div>
  );
}

function MenuFrame() {
  // A menu ignores the value it mounts with; the next value opens it once.
  const [request, setRequest] = useState(0);
  useLayoutEffect(() => setRequest(1), []);
  return (
    <div className="gallery-frame-menu">
      <BrowserMenu
        label="Page actions"
        description="Applies to this page"
        items={menuItems}
        onSelect={noop}
        openRequest={request}
      />
    </div>
  );
}

function NotificationsFrame() {
  const events = useMemo<BrowserNotification[]>(
    () => [
      {
        id: 'n1',
        destination: 'Chat',
        firstLine: 'ux-4 replied in Chat',
        quote: 'The gallery is up on 5183.',
        onActivate: noop,
      },
      {
        id: 'n2',
        destination: 'Thread',
        firstLine: 'core-ops answered your thread',
        onActivate: noop,
      },
    ],
    [],
  );
  return (
    <BrowserNotifications
      pageKey="gallery"
      events={events}
      suspended={false}
      isEligible={() => true}
    />
  );
}

function PanelReply({ send }: { send(): Promise<void> }) {
  const [draft, setDraft] = useState('');
  const [status, setStatus] = useState<'idle' | 'pending' | 'failed'>('idle');
  return (
    <>
      <BrowserField
        controlId="panel-persistent-reply"
        label="Reply draft"
        renderControl={(props) => (
          <textarea {...props} value={draft} onChange={(event) => setDraft(event.target.value)} />
        )}
      />
      <BrowserAction
        type="button"
        label="Send reply"
        variant="primary"
        busy={status === 'pending'}
        onActivate={async () => {
          setStatus('pending');
          try {
            await send();
            setStatus('idle');
          } catch {
            setStatus('failed');
          }
        }}
      />
      <p role="status">{status === 'failed' ? 'Reply failed. Your draft is retained.' : status}</p>
    </>
  );
}

function PanelFrame({ params }: { params: URLSearchParams }) {
  const [open, setOpen] = useState(params.get('open') !== 'false');
  const hostClose = params.get('host-close');
  const keepMounted = params.get('keep-mounted') === 'true';
  const failReply = useRef<(() => void) | null>(null);
  const [width, setWidth] = useState(window.innerWidth);
  const [refusals, setRefusals] = useState(0);
  const [pageActions, setPageActions] = useState(0);
  const [showOpener, setShowOpener] = useState(true);
  useEffect(() => {
    const resize = () => setWidth(window.innerWidth);
    window.addEventListener('resize', resize);
    return () => window.removeEventListener('resize', resize);
  }, []);
  const mode = width >= 1180 ? 'column' : width > 760 ? 'card' : 'sheet';
  return (
    <div className="gallery-panel-demo" data-mode={mode} data-open={open || undefined}>
      <div className="gallery-panel-page">
        <h1>Ops UI proposal</h1>
        {showOpener && (
          <BrowserAction
            type="button"
            variant="secondary"
            label="Discussion"
            onActivate={() => setOpen(true)}
          />
        )}
        <p>The page remains available while its discussion is open.</p>
        <BrowserAction
          type="button"
          variant="secondary"
          label="Page action"
          onActivate={() => {
            setPageActions((n) => n + 1);
            if (hostClose === 'close' || hostClose === 'unmount') setOpen(false);
          }}
        />
        {keepMounted && (
          <BrowserAction
            type="button"
            label="Fail pending reply"
            variant="secondary"
            onActivate={() => failReply.current?.()}
          />
        )}
        <p id="page-action-status">Page actions {pageActions}</p>
        <p role="status">{refusals ? `Dismissal refused ${refusals}` : ''}</p>
      </div>
      <div className="gallery-panel-column">
        {(open || hostClose !== 'unmount') && (
          <BrowserPanel
            open={open}
            keepMounted={keepMounted}
            title="Discussion"
            caption="1 / 2"
            headerActions={
              params.has('header-action') ? (
                <BrowserAction type="button" variant="primary" label="Save" onActivate={noop} />
              ) : undefined
            }
            mode={mode}
            focusFallback={() =>
              Array.from(document.querySelectorAll<HTMLButtonElement>('button')).find(
                (node) => node.textContent === 'Page action',
              ) ?? null
            }
            onDismiss={() => {
              if (params.get('refuse') === 'true') setRefusals((n) => n + 1);
              else setOpen(false);
            }}
          >
            <BrowserAction
              type="button"
              variant="text"
              label="Remove opener"
              onActivate={() => setShowOpener(false)}
            />
            {keepMounted && (
              <PanelReply
                send={() =>
                  new Promise<void>((_, reject) => {
                    failReply.current = () => {
                      failReply.current = null;
                      reject(new Error('Fixture reply failure'));
                    };
                  })
                }
              />
            )}
            <p>Navigation feedback</p>
            <blockquote>The page decides its shape.</blockquote>
            <BrowserField
              controlId="panel-reply"
              label="Reply"
              renderControl={(props) => (
                <textarea
                  {...props}
                  onKeyDown={(event) => {
                    if (event.key === 'Escape') event.preventDefault();
                  }}
                />
              )}
            />
            <BrowserAction type="button" variant="secondary" label="Reply" onActivate={() => {}} />
          </BrowserPanel>
        )}
      </div>
    </div>
  );
}

function GlyphFrame() {
  const host = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const root = host.current!;
    for (const size of [14, 24]) {
      const row = document.createElement('p');
      row.style.fontSize = `${size}px`;
      row.style.color = 'var(--tmt-ui-color-link)';
      row.dataset.size = String(size);
      for (const name of ['link', 'info'] as const) {
        const sample = document.createElement('span');
        sample.dataset.glyph = name;
        sample.append(createIcon(name), ` ${name} `);
        row.append(sample);
      }
      root.append(row);
    }
    return () => root.replaceChildren();
  }, []);
  return <main className="gallery-main" ref={host} aria-label="Shared glyphs" />;
}

export function frameFor(name: string, params: URLSearchParams): ReactNode {
  switch (name) {
    case 'glyphs':
      return <GlyphFrame />;
    case 'notice':
      return (
        <main className="gallery-main">
          <NoticeSection />
        </main>
      );
    case 'charts':
      return <ChartsSection />;
    case 'prose':
      return (
        <main className="gallery-main">
          <EditorialSection />
        </main>
      );
    case 'rail':
      return <RailFrame params={params} />;
    case 'shell':
      return <ShellFrame params={params} />;
    case 'hosted':
      return <HostedFrame />;
    case 'dialog':
      return <DialogFrame params={params} />;
    case 'panel':
      return <PanelFrame params={params} />;
    case 'drawer':
      return <DrawerFrame params={params} />;
    case 'source':
      return <SourceFrame params={params} />;
    case 'compact-controls':
      return <CompactControlsFrame params={params} />;
    case 'menu-host':
      return <MenuHostFrame params={params} />;
    case 'topbar':
      return <TopBarFrame params={params} />;
    case 'menu':
      return <MenuFrame />;
    case 'notifications':
      return <NotificationsFrame />;
    case 'theme-b':
      return <ThemeBFrame params={params} />;
    case 'ui-overlay':
      return <UiOverlayFrame params={params} />;
    default:
      return <p>Unknown frame {name}</p>;
  }
}

export function ShellSection() {
  return (
    <Section id="shell">
      <FramePair title="Shared common top bar · React adapter" frame="topbar" height={400} />
      <FramePair
        title="Shared common top bar · native renderer"
        frame="topbar"
        params={{ renderer: 'native' }}
        height={400}
      />
      <FramePair
        title="BrowserShell · rail expanded with module menu"
        note="mountShell renders the rail and module menu; BrowserShell hosts React and its header through useShellBar."
        frame="shell"
        height={420}
        width={1080}
      />
      <FramePair
        title="HostedShell · a route module inside the one TMT app"
        note="startShellHost owns the one rail and router; HostedShell gives the module's header slot and menu to the host."
        frame="hosted"
        height={320}
        width={1080}
      />
      <FramePair
        title="Rail counts · expanded (Colab 3, Remote 12, Ops 0 shows nothing)"
        note="A needs-you count is a number pill right-aligned on its row; zero draws nothing. The item's name carries it: Colab, 3 need you."
        frame="shell"
        height={400}
        width={920}
        params={{ counts: '12' }}
      />
      <FramePair
        title="Without a module menu"
        frame="shell"
        height={320}
        width={1080}
        params={{ menu: 'none' }}
      />
      <FramePair
        title="Rail tree · squads with member children"
        note="One nested level with a host-owned disclosure. The collapsed night squad holds the current member, so it shows as the selected row. Unread is bold, never a colour."
        frame="rail"
        height={560}
        width={1080}
        params={{ tree: 'true' }}
      />
      <FramePair title="Phone (390 px, ≤900 px layout)" frame="shell" width={390} height={560} />
      <FramePair
        title="Phone with the navigation drawer open"
        frame="shell"
        width={390}
        height={560}
        params={{ drawer: 'open' }}
      />
    </Section>
  );
}

export function OverlaysSection() {
  return (
    <Section id="overlays">
      <FramePair title="BrowserDialog" frame="dialog" height={360} />
      <FramePair
        title="BrowserDialog · wide"
        frame="dialog"
        params={{ size: 'wide' }}
        height={360}
      />
      <FramePair
        title="BrowserDialog · archive confirmation"
        frame="dialog"
        params={{ presentation: 'confirm' }}
        height={360}
      />
      <FramePair
        title="BrowserDialog · delete confirmation"
        frame="dialog"
        params={{ presentation: 'confirm', intent: 'delete' }}
        height={360}
      />
      <FramePair title="BrowserPanel · non-modal card" frame="panel" width={1024} height={440} />
      <FramePair title="BrowserPanel · non-modal sheet" frame="panel" width={390} height={500} />
      <FramePair title="BrowserDrawer" frame="drawer" height={420} />
      <FramePair title="BrowserDrawer · host header Save" frame="source" height={420} />
    </Section>
  );
}

export function NotificationsSection() {
  return (
    <Section id="notifications">
      <FramePair title="BrowserNotifications" frame="notifications" height={300} />
    </Section>
  );
}
