import { useEffect, useRef, useState } from 'react';
import {
  createHashHistory,
  createBrowserHistory,
  createRootRouteWithContext,
  createRoute,
  createRouter,
  Link,
  Outlet,
} from '@tanstack/react-router';
import type { PageView, PageTransport } from './transport.js';
import { mountRenderer } from './renderer.js';
import type { RenderState } from './renderer.js';
import { text } from './strings.js';
import { ExportPanel } from './export-panel.js';
import { AskControl, AskPanel } from './ask-panel.js';

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

function Shell() {
  const [dark, setDark] = useState(matchMedia('(prefers-color-scheme: dark)').matches);
  useEffect(() => {
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  }, [dark]);
  return (
    <>
      <header className="masthead">
        <Link className="brand" to="/">
          {text.product}
          <span>tmt</span>
        </Link>
        <span className="local">
          {location.pathname.startsWith('/r/') ? text.mounted : text.local}
        </span>
        <button className="theme" aria-label={text.theme} onClick={() => setDark(!dark)}>
          {dark ? '◐' : '◑'}
        </button>
      </header>
      <main>
        <Outlet />
      </main>
      <footer>{location.pathname.startsWith('/r/') ? text.mountedNote : text.adapter}</footer>
    </>
  );
}
function Home() {
  const space = home.useLoaderData();
  return (
    <section className="home">
      <p className="eyebrow">{text.pages}</p>
      <h1>{space.title}</h1>
      <p className="intro">{text.intro}</p>
      {space.pages.length ? (
        <ul className="pages">
          {space.pages.map((p) => (
            <li key={p.id}>
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
            </li>
          ))}
        </ul>
      ) : (
        <p>{text.empty}</p>
      )}
    </section>
  );
}
function Page() {
  const snapshot = page.useLoaderData();
  const [showSource, setShowSource] = useState(false);
  const [view, setView] = useState<PageView>({
    source: snapshot.source,
    title: snapshot.title,
    ownData: snapshot.ownData ?? false,
    own: snapshot.own,
    asks: snapshot.asks,
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
      ownData: snapshot.ownData ?? false,
      own: snapshot.own,
      asks: snapshot.asks,
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
  const [selection, setSelection] = useState('');
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
    void mountRenderer(host.current!, view.source, {
      signal: controller.signal,
      onState: setState,
      onSelection: setSelection,
    }).catch(() => {
      if (!controller.signal.aborted) setState('failed');
    });
    return () => controller.abort();
  }, [view.source, liveError]);
  return (
    <section className="page">
      <div className="page-bar">
        <Link className="back" to="/" aria-label={text.home}>
          ←
        </Link>
        <h1>{view.title || snapshot.title}</h1>
        <span className="chip">{text[snapshot.sharing]}</span>
        <span className={`status ${state === 'ready' ? 'live' : ''}`}>
          <span aria-hidden>{state === 'ready' ? '●' : state === 'loading' ? '○' : '✗'}</span>{' '}
          {state === 'ready' ? text.loaded : state === 'loading' ? text.loading : text.blocked}
        </span>
        <button aria-pressed={showSource} onClick={() => setShowSource(!showSource)}>
          {text.source}
        </button>
      </div>
      {view.ownData && <p role="status">{text.ownNotDisplayed}</p>}
      <AskControl
        key={`ask-control:${snapshot.id}`}
        binding={snapshot.binding?.ask}
        selection={selection}
        title={view.title || snapshot.title}
        blocked={!!liveError || state !== 'ready'}
      />
      <ExportPanel key={`export:${snapshot.id}`} binding={snapshot.binding} blocked={!!liveError} />
      <div className={`workspace ${showSource ? 'split' : ''}`}>
        {showSource && (
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
        )}
        <div className="canvas">
          <div className="boundary">
            <span>{text.boundary}</span>
          </div>
          <div className="frame-host" ref={host} />
          {(state === 'navigation' || state === 'failed') && (
            <div className="notice" role="alert">
              <h2>{text.blocked}</h2>
              <p>{liveError ?? (state === 'navigation' ? text.navigation : text.failed)}</p>
              <p>{text.limit}</p>
            </div>
          )}
        </div>
      </div>
      {view.askUnavailable && <p role="status">{text.askObservationUnavailable}</p>}
      {view.asks && (
        <AskPanel
          key={`ask-panel:${snapshot.id}`}
          records={view.asks}
          binding={snapshot.binding?.ask}
          blocked={!!liveError}
        />
      )}
      <p className="isolation-note">{text.warning}</p>
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
