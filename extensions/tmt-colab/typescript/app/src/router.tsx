import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import {
  createHashHistory,
  createBrowserHistory,
  createRootRouteWithContext,
  createRoute,
  createRouter,
  Link,
  Outlet,
  useRouter,
  useRouterState,
} from '@tanstack/react-router';
import type { PageView, PageTransport } from './transport.js';
import { AnnotationInput } from './annotation-input.js';
import { ThreadPanel } from './thread-panel.js';
import type { QuoteSelector, DiscussionRef } from './thread-records.js';
import { ShareDialog } from './share-dialog.js';
import { mountRenderer } from './renderer.js';
import type { RenderState, SelectionRect } from './renderer.js';
import { text } from './strings.js';
import { ExportPanel } from './export-panel.js';
import { PageDrawer } from './page-drawer.js';
import { AskControl, AskPanel } from './ask-panel.js';

function SelectionAnnotation({
  host,
  rectangle,
  inset,
  open,
  children,
}: {
  host: HTMLDivElement | null;
  rectangle: SelectionRect | null;
  inset: number;
  open(): void;
  children?: React.ReactNode;
}) {
  const expanded = children !== undefined;
  const element = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null);
  useEffect(() => {
    const place = () => {
      const frame = host?.querySelector('iframe')?.getBoundingClientRect();
      if (!frame || !rectangle) {
        setPosition(null);
        return;
      }
      const top = frame.top + Math.max(0, Math.min(frame.height, rectangle.y)),
        bottom = frame.top + Math.max(0, Math.min(frame.height, rectangle.y + rectangle.height)),
        left = frame.left + Math.max(0, Math.min(frame.width, rectangle.x)),
        right = frame.left + Math.max(0, Math.min(frame.width, rectangle.x + rectangle.width));
      if (bottom < inset || top > innerHeight) {
        setPosition(null);
        return;
      }
      const width = expanded ? Math.min(380, innerWidth - 24) : 100;
      const height = element.current?.offsetHeight ?? (expanded ? 200 : 38);
      const beside = !expanded && right + width + 8 <= Math.min(frame.right, innerWidth - 8);
      const below = bottom + height + 8 <= innerHeight - 8;
      setPosition({
        left: Math.max(
          8,
          Math.min(innerWidth - width - 8, frame.right - width - 8, beside ? right + 8 : left),
        ),
        top: Math.max(
          inset + 4,
          Math.min(innerHeight - height - 8, beside ? top : below ? bottom + 6 : top - height - 6),
        ),
      });
    };
    place();
    const observer = new ResizeObserver(place);
    if (element.current) observer.observe(element.current);
    window.addEventListener('scroll', place, { passive: true });
    window.addEventListener('resize', place);
    return () => {
      observer.disconnect();
      window.removeEventListener('scroll', place);
      window.removeEventListener('resize', place);
    };
  }, [host, rectangle, inset, expanded]);
  return (
    <div
      ref={element}
      className={children ? 'annotation-popover' : 'selection-control'}
      style={{ ...position, visibility: position ? 'visible' : 'hidden' }}
    >
      {children ?? (
        <button
          className="selection-ask"
          data-testid="selection-ask"
          onPointerDown={(event) => event.preventDefault()}
          onClick={(event) => {
            if (event.isTrusted) open();
          }}
        >
          Annotate
        </button>
      )}
    </div>
  );
}
const managementChanged = 'Management changed. Reopen the page to load its latest state.';

const root = createRootRouteWithContext<{ transport: PageTransport }>()({
  component: Shell,
  errorComponent: ({ error }) => (
    <section className="notice">
      <h1>{text.error}</h1>
      <p role="alert">{error instanceof Error ? error.message : text.blocked}</p>
      <Link to="/">{text.retry}</Link>
    </section>
  ),
  notFoundComponent: () => (
    <section className="notice">
      <h1>{text.error}</h1>
      <Link to="/">{text.retry}</Link>
    </section>
  ),
});
const home = createRoute({
  getParentRoute: () => root,
  path: '/',
  loader: ({ context }) => context.transport.spaceHome(),
  component: Home,
});
const page = createRoute({
  getParentRoute: () => root,
  path: '/pages/$pageId',
  loader: ({ context, params, abortController }) =>
    context.transport.page(params.pageId, abortController.signal),
  component: Page,
});

const blocked = createRoute({
  getParentRoute: () => root,
  path: '/blocked',
  loader: () => {
    throw new Error(text.pinMismatch);
  },
});

export function AppHeader({ linked = true }: { linked?: boolean }) {
  const brand = (
    <>
      {text.product}
      <span>tmt</span>
    </>
  );
  return (
    <header className="masthead">
      {linked ? (
        <Link className="brand" to="/">
          {brand}
        </Link>
      ) : (
        <span className="brand">{brand}</span>
      )}
      <span className="local">
        {location.pathname.startsWith('/r/') ? text.mounted : text.local}
      </span>
      <ThemeButton />
    </header>
  );
}
function ThemeButton({ menuLabel = false }: { menuLabel?: boolean }) {
  const [dark, setDark] = useState(() =>
    document.documentElement.dataset.theme
      ? document.documentElement.dataset.theme === 'dark'
      : matchMedia('(prefers-color-scheme: dark)').matches,
  );
  useEffect(() => {
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  }, [dark]);
  return (
    <button className="theme" aria-label={text.theme} onClick={() => setDark(!dark)}>
      <span className="theme-symbol" aria-hidden>
        {dark ? '◐' : '◑'}
      </span>
      {menuLabel && <span className="theme-label">Theme: {dark ? 'dark' : 'light'}</span>}
    </button>
  );
}
function Shell() {
  const isPage = useRouterState({
    select: (state) => state.location.pathname.startsWith('/pages/'),
  });
  return (
    <>
      {!isPage && <AppHeader />}
      <main className={isPage ? 'page-main' : undefined}>
        <Outlet />
      </main>
      {!isPage && (
        <footer>{location.pathname.startsWith('/r/') ? text.mountedNote : text.adapter}</footer>
      )}
    </>
  );
}
function ManageButton({ pageId, changed }: { pageId: string; changed?(): void }) {
  const port = root.useRouteContext().transport.management;
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const touched = useRef(false);
  if (!port) return null;
  return (
    <>
      <button
        onClick={(event) => {
          if (event.isTrusted) setOpen(true);
        }}
      >
        Manage page
      </button>
      {open &&
        createPortal(
          <ShareDialog
            port={port}
            pageId={pageId}
            committed={() => {
              touched.current = true;
              changed?.();
            }}
            close={() => {
              setOpen(false);
              if (touched.current) {
                touched.current = false;
                void router.invalidate();
              }
            }}
          />,
          document.body,
        )}
    </>
  );
}
function Home() {
  const space = home.useLoaderData();
  const { transport } = root.useRouteContext();
  const [archived, setArchived] = useState(false);
  const pages = space.pages.filter((p) => Boolean(p.archived) === archived);
  return (
    <section className="home">
      <p className="eyebrow">{text.pages}</p>
      <h1>{space.title}</h1>
      <p className="intro">{text.intro}</p>
      {transport.management && (
        <label>
          Show archived pages
          <input
            type="checkbox"
            checked={archived}
            onChange={(e) => setArchived(e.target.checked)}
          />
        </label>
      )}
      {pages.length ? (
        <ul className="pages">
          {pages.map((p) => (
            <li key={p.id}>
              {p.archived ? (
                <div className="archived-page">
                  <h2>{p.title}</h2>
                  <span className="chip">Archived · writes frozen</span>
                </div>
              ) : (
                <Link to="/pages/$pageId" params={{ pageId: p.id }}>
                  <div>
                    <span className="page-mark">▤</span>
                    <h2>{p.title}</h2>
                  </div>
                  <span className="chip">{text[p.sharing]}</span>
                  <span className="open">
                    {text.open} <span aria-hidden>↗</span>
                  </span>
                </Link>
              )}
              {transport.management && (
                <p>
                  Retention: {p.retentionDays === null ? 'forever' : `${p.retentionDays} days`}.
                  Expiry time unavailable.
                </p>
              )}
              <ManageButton pageId={p.id} />
            </li>
          ))}
        </ul>
      ) : (
        <p>
          {archived ? 'No archived pages.' : space.pages.length ? 'No active pages.' : text.empty}
        </p>
      )}
    </section>
  );
}
function Page() {
  const { transport } = root.useRouteContext();
  const snapshot = page.useLoaderData();
  const backendName = transport.backendName?.trim();
  const backendLabel = backendName ? `local · ${backendName}` : 'local';
  const [panel, setPanel] = useState<'source' | 'comments' | 'ask' | 'export' | null>(null);
  const [menu, setMenu] = useState(false);
  const toolbar = useRef<HTMLElement>(null);
  useEffect(() => {
    if (!menu) return;
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !toolbar.current?.contains(event.target)) setMenu(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setMenu(false);
        toolbar.current?.querySelector<HTMLButtonElement>('.page-overflow-toggle')?.focus();
      }
    };
    document.addEventListener('pointerdown', outside);
    document.addEventListener('keydown', escape);
    return () => {
      document.removeEventListener('pointerdown', outside);
      document.removeEventListener('keydown', escape);
    };
  }, [menu]);
  const toggle = (value: typeof panel) => {
    setMenu(false);
    setPanel((previous) => (previous === value ? null : value));
  };
  const [view, setView] = useState<PageView>({
    source: snapshot.source,
    title: snapshot.title,
    publisherAgent: snapshot.publisherAgent,
    ownData: snapshot.ownData ?? false,
    own: snapshot.own,
    asks: snapshot.asks,
    threads: snapshot.threads,
  });
  const latest = useRef(view),
    dirty = useRef(false),
    base = useRef(snapshot.source);
  const [draft, setDraft] = useState(snapshot.source),
    [saving, setSaving] = useState(false);
  const [liveError, setLiveError] = useState<string | null>(null),
    [editError, setEditError] = useState<string | null>(null);
  useEffect(() => {
    dirty.current = false;
    base.current = snapshot.source;
    setDraft(snapshot.source);
    setView({
      source: snapshot.source,
      title: snapshot.title,
      publisherAgent: snapshot.publisherAgent,
      ownData: snapshot.ownData ?? false,
      own: snapshot.own,
      asks: snapshot.asks,
      threads: snapshot.threads,
    });
    setLiveError(null);
    setEditError(null);
    const unsubscribe = snapshot.binding?.subscribe(
      (value) => {
        const next = { ...value, ownData: value.ownData ?? false };
        latest.current = next;
        setView(next);
        if (!dirty.current) {
          base.current = value.source;
          setDraft(value.source);
        }
      },
      (error) => setLiveError(error.message),
    );
    return () => {
      unsubscribe?.();
    };
  }, [snapshot]);
  async function save() {
    if (!snapshot.binding || saving) return;
    setSaving(true);
    setEditError(null);
    try {
      await snapshot.binding.edit(draft, base.current);
      dirty.current = false;
      base.current = latest.current.source;
      setDraft(latest.current.source);
    } catch {
      setEditError(text.editFailed);
    } finally {
      setSaving(false);
    }
  }
  const [reconnecting, setReconnecting] = useState(false);
  const [reconnectFailed, setReconnectFailed] = useState(false);
  async function reconnect() {
    if (reconnecting) return;
    setReconnecting(true);
    try {
      if (!(await snapshot.binding?.reconnect?.())) setReconnectFailed(true);
    } catch {
      setReconnectFailed(true);
    }
  }
  const [selection, setSelection] = useState('');
  const [selector, setSelector] = useState<QuoteSelector | null>(null);
  const currentSelector = useRef<QuoteSelector | null>(null);
  const [rectangle, setRectangle] = useState<SelectionRect | null>(null);
  const currentRectangle = useRef<SelectionRect | null>(null);
  const [annotation, setAnnotation] = useState<{
    selector: QuoteSelector;
    rectangle: SelectionRect;
  }>();
  const [activeThread, setActiveThread] = useState<string | null>(null);
  useEffect(() => {
    setAnnotation(undefined);
    setActiveThread(null);
    setPanel(null);
  }, [snapshot.id]);
  function annotate() {
    if (!currentSelector.current || !currentRectangle.current) return;
    setAnnotation({
      selector: structuredClone(currentSelector.current),
      rectangle: { ...currentRectangle.current },
    });
    setActiveThread(null);
    setRectangle(null);
    setPanel(null);
    setMenu(false);
  }
  function cancelAnnotation() {
    setAnnotation(undefined);
    setRectangle(currentRectangle.current);
  }
  function openThread(ref: DiscussionRef | null) {
    if (!ref) {
      setActiveThread(null);
      return;
    }
    const id = `${ref.writer}:${ref.id}`;
    setAnnotation(undefined);
    setActiveThread(id);
    setPanel('comments');
    setMenu(false);
    renderer.current?.scrollAnchor(id);
  }
  const [resolved, setResolved] = useState<string[]>([]);
  const renderer = useRef<Awaited<ReturnType<typeof mountRenderer>> | null>(null);
  const [state, setState] = useState<RenderState | 'loading'>('loading');
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const controller = new AbortController();
    if (liveError) {
      setState('failed');
      return () => controller.abort();
    }
    setState('loading');
    setSelection('');
    setSelector(null);
    currentSelector.current = null;
    currentRectangle.current = null;
    setAnnotation(undefined);
    setRectangle(null);
    setResolved([]);
    void mountRenderer(host.current!, view.source, {
      signal: controller.signal,
      onState: setState,
      viewportInset: () =>
        (toolbar.current?.offsetHeight ?? 56) + (toolbar.current?.getBoundingClientRect().top ?? 0),
      onSelection: (value, quote, rect) => {
        setSelection(value);
        setSelector(quote ?? null);
        currentSelector.current = quote ?? null;
        currentRectangle.current = rect ?? null;
        setRectangle(rect ?? null);
      },
      onAnchors: setResolved,
      onAnnotate: annotate,
      onOpenThread: (id) => {
        const thread = latest.current.threads?.find(
          (value) => `${value.ref.writer}:${value.threadId}` === id && !value.deleted,
        );
        if (thread) openThread(thread.ref);
      },
    })
      .then((handle) => {
        if (controller.signal.aborted) handle.destroy();
        else renderer.current = handle;
      })
      .catch(() => {
        if (!controller.signal.aborted) setState('failed');
      });
    return () => {
      controller.abort();
      renderer.current = null;
    };
  }, [view.source, liveError]);
  useEffect(() => {
    if (state !== 'ready') return;
    renderer.current?.highlight(
      (view.threads ?? []).flatMap((thread) =>
        !thread.deleted && thread.anchor
          ? [
              {
                id: `${thread.ref.writer}:${thread.threadId}`,
                selector: thread.anchor,
              },
            ]
          : [],
      ),
    );
  }, [state, view.threads]);
  return (
    <section className="page">
      <header className="page-bar" ref={toolbar} data-menu-open={menu}>
        <Link className="back" to="/" aria-label={text.home} title={text.product}>
          <span aria-hidden>←</span>
          <span className="page-brand">
            {text.product}
            <span className="page-tmt">tmt</span>
          </span>
        </Link>
        <h1 title={view.title || snapshot.title}>{view.title || snapshot.title}</h1>
        <span className="page-backend" title={backendLabel}>
          {backendLabel}
        </span>
        <span className="chip page-sharing">{text[snapshot.sharing]}</span>
        <span
          className={`status ${state === 'ready' ? 'live' : ''}`}
          title={
            state === 'ready' ? text.loaded : state === 'loading' ? text.loading : text.blocked
          }
        >
          <span aria-hidden>{state === 'ready' ? '●' : state === 'loading' ? '○' : '✗'}</span>
          <span className="status-label">
            {state === 'ready' ? text.loaded : state === 'loading' ? text.loading : text.blocked}
          </span>
        </span>
        <button
          className="page-overflow-toggle"
          aria-label="More page actions"
          aria-expanded={menu}
          onClick={(event) => {
            if (event.isTrusted) setMenu(!menu);
          }}
        >
          •••
        </button>
        <div
          className="page-secondary"
          role="group"
          aria-label="Page actions"
          onClick={(event) => {
            if (
              event.isTrusted &&
              event.target instanceof Element &&
              event.target.closest('button')
            )
              setMenu(false);
          }}
        >
          <button
            className="page-menu-close"
            aria-label="Close page actions"
            onClick={() => setMenu(false)}
          >
            ×
          </button>
          <div className="page-menu-meta">
            <span title={backendLabel}>{backendLabel}</span>
            <span>{text[snapshot.sharing]}</span>
          </div>
          <button
            data-testid="ask-toggle"
            aria-label="Agent conversations"
            aria-expanded={panel === 'ask'}
            onClick={(event) => {
              if (event.isTrusted) toggle('ask');
            }}
          >
            {text.askShort}
          </button>
          <button
            data-testid="comments-toggle"
            aria-expanded={panel === 'comments'}
            onClick={(event) => {
              if (event.isTrusted) toggle('comments');
            }}
          >
            {text.comments}
          </button>
          <button
            aria-pressed={panel === 'source'}
            onClick={(event) => {
              if (event.isTrusted) toggle('source');
            }}
          >
            {text.source}
          </button>
          <ExportPanel
            key={`export:${snapshot.id}`}
            binding={snapshot.binding}
            blocked={!!liveError}
            drawer
            opened={panel === 'export'}
            changeOpen={(open) => {
              setMenu(false);
              setPanel(open ? 'export' : null);
            }}
          />
          <ManageButton
            pageId={snapshot.id}
            changed={() => {
              snapshot.binding?.close();
              setLiveError(managementChanged);
            }}
          />
          <ThemeButton menuLabel />
          <details className="page-information">
            <summary aria-label="Page information">
              <span className="info-symbol" aria-hidden>
                ⓘ
              </span>
              <span className="info-label">Page information</span>
            </summary>
            <p>{text.warning}</p>
          </details>
        </div>
      </header>
      <div className="workspace">
        <div className="canvas">
          <div className="frame-host" ref={host} />
          {(state === 'navigation' || state === 'failed') && (
            <div className="notice" role="alert">
              <h2>{text.blocked}</h2>
              <p>{liveError ?? (state === 'navigation' ? text.navigation : text.failed)}</p>
              <p>{text.limit}</p>
              {liveError === 'Sync disconnected' && snapshot.binding?.reconnect && (
                <button
                  disabled={reconnecting}
                  data-testid="colab-reconnect"
                  onClick={(event) => {
                    if (event.isTrusted) void reconnect();
                  }}
                >
                  {text.reconnect}
                </button>
              )}
              {reconnectFailed && <p>{text.reconnectFailed}</p>}
            </div>
          )}
        </div>
      </div>
      {snapshot.binding?.discussion && snapshot.binding?.ask && !liveError && state === 'ready' && (
        <SelectionAnnotation
          host={host.current}
          rectangle={annotation?.rectangle ?? rectangle}
          inset={toolbar.current?.offsetHeight ?? 56}
          open={annotate}
        >
          {annotation && (
            <section className="annotation-new" role="dialog" aria-label="Annotate selection">
              <blockquote>{annotation.selector.exact}</blockquote>
              <AnnotationInput
                key={JSON.stringify(annotation.selector)}
                binding={snapshot.binding.ask}
                discussion={snapshot.binding.discussion}
                anchor={annotation.selector}
                asks={view.asks ?? []}
                title={view.title || snapshot.title}
                publisher={view.publisherAgent}
                blocked={!!liveError || state !== 'ready'}
                cancel={cancelAnnotation}
                committed={openThread}
              />
            </section>
          )}
        </SelectionAnnotation>
      )}
      <PageDrawer
        open={panel === 'source'}
        title={text.source}
        kind="source"
        close={() => setPanel(null)}
      >
        <div className="source">
          <span>
            <label htmlFor="source-edit">{text.source}</label>
            {snapshot.binding && (
              <button
                disabled={saving || !!liveError || draft === base.current}
                onClick={() => void save()}
              >
                {saving ? text.saving : text.save}
              </button>
            )}
          </span>
          <textarea
            id="source-edit"
            readOnly={!snapshot.binding || saving || !!liveError}
            spellCheck={false}
            value={draft}
            onChange={(event) => {
              dirty.current = true;
              setDraft(event.target.value);
            }}
          />
          {editError && <p role="alert">{editError}</p>}
        </div>
      </PageDrawer>
      <PageDrawer
        open={panel === 'comments'}
        title={text.comments}
        kind="comments"
        close={() => setPanel(null)}
      >
        <ThreadPanel
          hideHeader
          key={`discussion:${snapshot.id}`}
          threads={view.threads ?? []}
          resolved={resolved}
          selection={selector}
          binding={liveError === managementChanged ? undefined : snapshot.binding?.discussion}
          ask={liveError === managementChanged ? undefined : snapshot.binding?.ask}
          title={view.title || snapshot.title}
          publisher={view.publisherAgent}
          asks={view.asks ?? []}
          active={activeThread}
          select={openThread}
          blocked={!!liveError || state !== 'ready'}
        />
      </PageDrawer>
      <PageDrawer open={panel === 'ask'} title={text.ask} kind="ask" close={() => setPanel(null)}>
        <AskControl
          key={`ask-control:${snapshot.id}`}
          binding={liveError === managementChanged ? undefined : snapshot.binding?.ask}
          selection={selection}
          title={view.title || snapshot.title}
          blocked={!!liveError || state !== 'ready'}
        />
        {view.askUnavailable && <p role="status">{text.askObservationUnavailable}</p>}
        <AskPanel
          key={`ask-panel:${snapshot.id}`}
          records={view.asks ?? []}
          binding={liveError === managementChanged ? undefined : snapshot.binding?.ask}
          blocked={!!liveError}
        />
      </PageDrawer>
    </section>
  );
}
export function createAppRouter(transport: PageTransport, space?: string) {
  const history = space
    ? createBrowserHistory({
        parseLocation: () => {
          const fragment = new URLSearchParams(location.hash.slice(1));
          let path = fragment.get('space') === space ? (fragment.get('path') ?? '/') : '/blocked';
          if (!/^\/(?:pages\/[0-9a-f-]+)?$/.test(path)) path = '/blocked';
          return {
            href: path,
            pathname: path,
            search: '',
            hash: '',
            state: { ...window.history.state, __TSR_index: window.history.state?.__TSR_index ?? 0 },
          };
        },
        createHref: (path) =>
          `${location.pathname}#space=${space}${path === '/' ? '' : `&path=${encodeURIComponent(path)}`}`,
      })
    : createHashHistory();
  return createRouter({
    routeTree: root.addChildren([home, page, blocked]),
    history,
    context: { transport },
  });
}
declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof createAppRouter>;
  }
}
