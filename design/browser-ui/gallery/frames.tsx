import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Search, Settings } from 'lucide-react';
import {
  BrowserAction,
  BrowserDialog,
  BrowserDrawer,
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
import { menuItems } from './components.js';
import { UiOverlayFrame } from './ui-overlays.js';

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
  const view = useMemo<ShellView>(() => {
    const place = { origin, href: `${origin}/colab/pages` };
    const colab = answer.modules[0]!;
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
  }, [params]);
  useEffect(() => {
    // The rail's own toggle remembers a viewer's choice; the gallery pins each frame's state.
    const shell = document.querySelector<HTMLElement>('.tmt-ui-shell')!;
    shell.dataset.rail = params.get('rail') ?? 'expanded';
    if (params.get('drawer') === 'open')
      shell.querySelector<HTMLButtonElement>('.tmt-ui-shell-open')!.click();
  }, [params]);
  return (
    <BrowserShell view={view} select={() => true}>
      <ShellHeader />
      <div className="gallery-frame-main">
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

function DialogFrame() {
  const [open, setOpen] = useState(true);
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
        title="Move to project"
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
              label="Move"
              onActivate={() => setOpen(false)}
            />
          </>
        }
      >
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
      </BrowserDialog>
    </>
  );
}

function DrawerFrame({ params }: { params: URLSearchParams }) {
  const [open, setOpen] = useState(true);
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

export function frameFor(name: string, params: URLSearchParams): ReactNode {
  switch (name) {
    case 'shell':
      return <ShellFrame params={params} />;
    case 'hosted':
      return <HostedFrame />;
    case 'dialog':
      return <DialogFrame />;
    case 'drawer':
      return <DrawerFrame params={params} />;
    case 'menu':
      return <MenuFrame />;
    case 'notifications':
      return <NotificationsFrame />;
    case 'ui-overlay':
      return <UiOverlayFrame params={params} />;
    default:
      return <p>Unknown frame {name}</p>;
  }
}

export function ShellSection() {
  return (
    <Section id="shell">
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
        title="Rail collapsed"
        frame="shell"
        height={420}
        width={1080}
        params={{ rail: 'collapsed' }}
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
        title="Rail counts · collapsed"
        note="The count becomes a compact numeral at the icon's top-right corner, past the glyph, so every icon stays whole."
        frame="shell"
        height={400}
        width={920}
        params={{ counts: '12', rail: 'collapsed' }}
      />
      <FramePair
        title="Without a module menu"
        frame="shell"
        height={320}
        width={1080}
        params={{ menu: 'none' }}
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
      <FramePair title="BrowserDrawer" frame="drawer" height={420} />
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
