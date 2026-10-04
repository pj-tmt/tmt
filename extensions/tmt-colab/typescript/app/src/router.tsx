import { validPagePrefix } from './short-links.js';
import {
  FileText,
  ArrowUpRight,
  Circle,
  LoaderCircle,
  Ellipsis,
  Info,
  Moon,
  Sun,
  X,
} from 'lucide-react';
import { ColabHeader } from './colab-header.js';
import { NoticeCard } from './notice-card.js';
import { RetentionHint } from './retention-hint.js';
import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import {
  createHashHistory,
  createBrowserHistory,
  createRootRouteWithContext,
  createRoute,
  createRouter,
  Link,
  redirect,
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
import { ChatPanel } from './chat-panel.js';
import { isChatThread } from './thread-records.js';

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
    <>
      <AppHeader title={text.error} />
      <main>
        <NoticeCard
          state="blocked"
          eyebrow={text.product}
          title={text.error}
          actions={
            <Link className="notice-action" to="/">
              {text.retry}
            </Link>
          }
        >
          <p>{error instanceof Error ? error.message : text.blocked}</p>
        </NoticeCard>
      </main>
    </>
  ),
  notFoundComponent: () => (
    <>
      <NoticeCard
        state="blocked"
        eyebrow={text.product}
        title={text.error}
        actions={
          <Link className="notice-action" to="/">
            {text.retry}
          </Link>
        }
      />
    </>
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

const shortPage = createRoute({
  getParentRoute: () => root,
  path: '/short/$prefix',
  loader: async ({ context, params }) => {
    if (!validPagePrefix(params.prefix)) throw new Error('Page unavailable');
    const home = await context.transport.spaceHome();
    const known = home.pageIds ?? home.pages.map((page) => ({ pageId: page.id, deleted: false }));
    const pages = known
      .filter((page) => page.pageId.startsWith(params.prefix))
      .map((row) => ({
        id: row.pageId,
        deleted: row.deleted,
        title: row.deleted
          ? ''
          : (home.pages.find((page) => page.id === row.pageId)?.title ?? text.unknownPageTitle),
        archived: row.deleted ? false : home.pages.find((page) => page.id === row.pageId)?.archived,
      }));
    if (pages.length === 0) throw new Error('Page unavailable');
    if (pages.length === 1 && !pages[0].deleted)
      throw redirect({ to: '/pages/$pageId', params: { pageId: pages[0].id }, replace: true });
    return { pages, prefix: params.prefix };
  },
  component: ShortPageChoice,
});
function ShortPageChoice() {
  const { pages, prefix } = shortPage.useLoaderData();
  const deleted = pages.length === 1 && pages[0].deleted;
  return (
    <section className="notice short-page-choice">
      <h1>{deleted ? 'This page was deleted' : 'Choose a page'}</h1>
      {!deleted && <p>More than one page matches {prefix}. Choose the page you want to open.</p>}
      <ul>
        {pages.map((page) => (
          <li key={page.id}>
            {page.deleted ? (
              <span aria-disabled="true">
                Deleted page
                <br />
                <code>{page.id}</code>
              </span>
            ) : (
              <Link to="/pages/$pageId" params={{ pageId: page.id }}>
                {page.title}
                {page.archived ? ' · Archived' : ''}
                <br />
                <code>{page.id}</code>
              </Link>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}

const blocked = createRoute({
  getParentRoute: () => root,
  path: '/blocked',
  loader: () => {
    throw new Error(text.pinMismatch);
  },
});

export function AppHeader({
  linked = true,
  title = text.pages,
}: {
  linked?: boolean;
  title?: string;
}) {
  return (
    <ColabHeader
      title={title}
      home={
        linked
          ? (brand) => (
              <Link className="colab-brand" to="/" aria-label={text.home}>
                {brand}
              </Link>
            )
          : undefined
      }
      actions={<ThemeButton />}
    />
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
        {dark ? <Moon aria-hidden /> : <Sun aria-hidden />}
      </span>
      {menuLabel && <span className="theme-label">Theme: {dark ? 'dark' : 'light'}</span>}
    </button>
  );
}
function Shell() {
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const isPage = pathname.startsWith('/pages/');
  return (
    <>
      {!isPage && <AppHeader title={pathname === '/' ? text.pages : text.error} />}
      <main className={isPage ? 'page-main' : undefined}>
        <Outlet />
      </main>
      {!isPage && (
        <footer>{location.pathname.startsWith('/r/') ? text.mountedNote : text.adapter}</footer>
      )}
    </>
  );
}
function ManageButton({
  pageId,
  title,
  changed,
}: {
  pageId: string;
  title: string;
  changed?(): void;
}) {
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
            title={title}
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
  useEffect(() => {
    document.title =
      space.title === text.product ? text.product : `${space.title} · ${text.product}`;
  }, [space.title]);
  return (
    <section className="home">
      <p className="eyebrow">{text.pages}</p>
      <h1>{space.title}</h1>
      <p className="intro">{text.intro}</p>
      {transport.management && (
        <button
          type="button"
          className="archive-toggle"
          aria-pressed={archived}
          onClick={(event) => {
            if (event.isTrusted) setArchived(!archived);
          }}
        >
          <span className="toggle-box" aria-hidden="true" />
          Show archived
        </button>
      )}
      {pages.length ? (
        <ul className="pages">
          {pages.map((p) => (
            <li key={p.id} data-page-id={p.id} className="page-card">
              {p.archived ? (
                <div className="archived-page">
                  <FileText className="page-mark" aria-hidden />
                  <h2>{p.title.trim() || text.unknownPageTitle}</h2>
                  <span className="page-id">{p.id.slice(0, 8)}</span>
                  <span className="chip">Archived · writes frozen</span>
                  {transport.management && <RetentionHint page={p} />}
                </div>
              ) : (
                <Link className="page-card-link" to="/pages/$pageId" params={{ pageId: p.id }}>
                  <FileText className="page-mark" aria-hidden />
                  <h2>{p.title.trim() || text.unknownPageTitle}</h2>
                  <span className="page-id">{p.id.slice(0, 8)}</span>
                  <span className="chip">{text[p.sharing]}</span>
                  {transport.management && <RetentionHint page={p} />}
                  <span className="open">
                    {text.open}
                    <ArrowUpRight aria-hidden />
                  </span>
                </Link>
              )}
              {transport.management && (
                <div className="page-card-actions">
                  <details>
                    <summary>Details</summary>
                    <p className="management-id">Page ID: {p.id}</p>
                  </details>
                  <ManageButton pageId={p.id} title={p.title} />
                </div>
              )}
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
  const [panel, setPanel] = useState<'source' | 'comments' | 'chat' | 'export' | null>(null);
  const [menu, setMenu] = useState(false);
  const [chatOpened, setChatOpened] = useState(false);
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
    if (value === 'chat') setChatOpened(true);
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
  useEffect(() => {
    document.title = `${view.title || snapshot.title} · ${text.product}`;
    return () => {
      document.title = text.product;
    };
  }, [view.title, snapshot.title]);
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
  const [selector, setSelector] = useState<QuoteSelector | null>(null);
  const currentSelector = useRef<QuoteSelector | null>(null);
  const [rectangle, setRectangle] = useState<SelectionRect | null>(null);
  const currentRectangle = useRef<SelectionRect | null>(null);
  const [annotation, setAnnotation] = useState<{
    selector: QuoteSelector;
    rectangle: SelectionRect;
    /** Text kept from an earlier close of this selection. */
    restored?: string;
  }>();
  const annotationRef = useRef(annotation);
  annotationRef.current = annotation;
  const popover = useRef<HTMLElement>(null);
  // An unsent draft lives in memory for this page only: never stored, never sent to the frame.
  const drafts = useRef(new Map<string, string>());
  const annotationDraft = useRef('');
  const annotationBusy = useRef(false);
  const [activeThread, setActiveThread] = useState<string | null>(null);
  useEffect(() => {
    setAnnotation(undefined);
    setActiveThread(null);
    setPanel(null);
    setChatOpened(false);
  }, [snapshot.id]);
  function annotate() {
    if (!currentSelector.current || !currentRectangle.current) return;
    setAnnotation({
      selector: structuredClone(currentSelector.current),
      rectangle: { ...currentRectangle.current },
      restored: drafts.current.get(JSON.stringify(currentSelector.current)),
    });
    setActiveThread(null);
    setRectangle(null);
    setPanel(null);
    setMenu(false);
  }
  /** True once something beyond the prefilled `@agent` has been typed. */
  const typed = () =>
    annotationDraft.current.trim() !== '' && !/^@\S*\s*$/.test(annotationDraft.current);
  /** Closes without losing typed text: it comes back when the same selection is annotated again. */
  function closeAnnotation(focusPage: boolean) {
    // A send in flight is not interrupted by the ×, Escape, an outside press or a cleared selection.
    if (!annotation || annotationBusy.current) return;
    const key = JSON.stringify(annotation.selector);
    if (typed()) drafts.current.set(key, annotationDraft.current);
    else drafts.current.delete(key);
    annotationDraft.current = '';
    setAnnotation(undefined);
    setRectangle(currentRectangle.current);
    if (focusPage) queueMicrotask(() => host.current?.querySelector('iframe')?.focus());
  }
  const closeRef = useRef(closeAnnotation);
  closeRef.current = closeAnnotation;
  const selectionCleared = useRef<() => void>(() => {});
  selectionCleared.current = () => {
    // A page click that clears the selection never hides a typed draft.
    if (annotation && !typed()) closeAnnotation(false);
  };
  useEffect(() => {
    if (!annotation) return;
    const outside = (event: PointerEvent) => {
      const target = event.target;
      if (
        target instanceof Element &&
        !popover.current?.contains(target) &&
        !target.closest('[role="listbox"]')
      )
        closeRef.current(false);
    };
    document.addEventListener('pointerdown', outside, true);
    return () => document.removeEventListener('pointerdown', outside, true);
  }, [annotation]);
  function openThread(ref: DiscussionRef | null) {
    if (!ref) {
      setActiveThread(null);
      return;
    }
    const id = `${ref.writer}:${ref.id}`;
    if (annotationRef.current)
      drafts.current.delete(JSON.stringify(annotationRef.current.selector));
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
        (toolbar.current?.offsetHeight ?? 0) + (toolbar.current?.getBoundingClientRect().top ?? 0),
      onSelection: (_value, quote, rect) => {
        setSelector(quote ?? null);
        currentSelector.current = quote ?? null;
        currentRectangle.current = rect ?? null;
        setRectangle(rect ?? null);
        if (!quote) selectionCleared.current();
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
      <ColabHeader
        headerRef={toolbar}
        menuOpen={menu}
        title={view.title || snapshot.title || text.unknownPageTitle}
        home={(brand) => (
          <Link className="colab-brand" to="/" aria-label={text.home}>
            {brand}
          </Link>
        )}
        actions={
          <>
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
              <span aria-hidden>
                {state === 'failed' || state === 'navigation' ? (
                  <X aria-hidden />
                ) : state === 'loading' ? (
                  <LoaderCircle aria-hidden />
                ) : (
                  <Circle fill="currentColor" aria-hidden />
                )}
              </span>
              <span className="status-label">
                {state === 'ready'
                  ? text.loaded
                  : state === 'loading'
                    ? text.loading
                    : text.blocked}
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
              <Ellipsis aria-hidden />
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
                <X aria-hidden />
              </button>
              <div className="page-menu-meta">
                <span title={backendLabel}>{backendLabel}</span>
                <span>{text[snapshot.sharing]}</span>
              </div>
              <button
                data-testid="chat-toggle"
                aria-label="Chat"
                aria-expanded={panel === 'chat'}
                onClick={(event) => {
                  if (event.isTrusted) toggle('chat');
                }}
              >
                Chat
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
                title={view.title || snapshot.title}
                changed={() => {
                  snapshot.binding?.close();
                  setLiveError(managementChanged);
                }}
              />
              <ThemeButton menuLabel />
              <details className="page-information">
                <summary aria-label="Page information">
                  <span className="info-symbol" aria-hidden>
                    <Info aria-hidden />
                  </span>
                  <span className="info-label">Page information</span>
                </summary>
                <p>{text.warning}</p>
              </details>
            </div>
          </>
        }
      />
      <div className="workspace">
        <div className="canvas">
          <div className="frame-host" ref={host} />
          {(state === 'navigation' || state === 'failed') && (
            <NoticeCard state="blocked" eyebrow={text.product} title={text.blocked}>
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
            </NoticeCard>
          )}
        </div>
      </div>
      {snapshot.binding?.discussion && snapshot.binding?.ask && !liveError && state === 'ready' && (
        <SelectionAnnotation
          host={host.current}
          rectangle={annotation?.rectangle ?? rectangle}
          inset={toolbar.current?.offsetHeight ?? 0}
          open={annotate}
        >
          {annotation && (
            <section
              className="annotation-new"
              role="dialog"
              aria-label="Annotate selection"
              ref={popover}
              onKeyDown={(event) => {
                if (event.key === 'Escape' && !event.defaultPrevented) {
                  event.preventDefault();
                  closeAnnotation(true);
                }
              }}
            >
              <button
                type="button"
                className="annotation-close"
                aria-label="Close annotation"
                onClick={(event) => {
                  if (event.isTrusted) closeAnnotation(true);
                }}
              >
                ×
              </button>
              <blockquote>{annotation.selector.exact}</blockquote>
              {annotation.restored !== undefined && <p className="annotation-hint">Draft kept</p>}
              <AnnotationInput
                key={JSON.stringify(annotation.selector)}
                binding={snapshot.binding.ask}
                discussion={snapshot.binding.discussion}
                anchor={annotation.selector}
                asks={view.asks ?? []}
                title={view.title || snapshot.title}
                publisher={view.publisherAgent}
                blocked={!!liveError || state !== 'ready'}
                initialValue={annotation.restored}
                onDraft={(value) => {
                  annotationDraft.current = value;
                }}
                onBusy={(busy) => {
                  annotationBusy.current = busy;
                }}
                cancel={() => closeAnnotation(true)}
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
          threads={(view.threads ?? []).filter((thread) => !isChatThread(thread))}
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
      <PageDrawer open={panel === 'chat'} title="Chat" kind="chat" close={() => setPanel(null)}>
        {view.askUnavailable && <p role="status">{text.askObservationUnavailable}</p>}
        {chatOpened && (
          <ChatPanel
            key={`chat:${snapshot.id}`}
            threads={view.threads ?? []}
            asks={view.asks ?? []}
            binding={liveError === managementChanged ? undefined : snapshot.binding?.ask}
            discussion={liveError === managementChanged ? undefined : snapshot.binding?.discussion}
            title={view.title || snapshot.title}
            publisher={view.publisherAgent}
            blocked={!!liveError || state !== 'ready'}
            close={() => setPanel(null)}
          />
        )}
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
          if (!/^\/(?:pages\/[0-9a-f-]+|short\/[0-9a-f-]{8,36})?$/.test(path)) path = '/blocked';
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
    routeTree: root.addChildren([home, page, shortPage, blocked]),
    history,
    context: { transport },
  });
}
declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof createAppRouter>;
  }
}
