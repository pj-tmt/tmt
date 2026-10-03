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
import type { PageTransport } from './transport.js';
import { mountRenderer } from './renderer.js';
import type { RenderState } from './renderer.js';
import { text } from './strings.js';

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
  const [state, setState] = useState<RenderState | 'loading'>('loading');
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const controller = new AbortController();
    setState('loading');
    void mountRenderer(host.current!, snapshot.source, {
      signal: controller.signal,
      onState: setState,
    }).catch(() => {
      if (!controller.signal.aborted) setState('failed');
    });
    return () => controller.abort();
  }, [snapshot]);
  return (
    <section className="page">
      <div className="page-bar">
        <Link className="back" to="/" aria-label={text.home}>
          ←
        </Link>
        <h1>{snapshot.title}</h1>
        <span className="chip">{text[snapshot.sharing]}</span>
        <span className={`status ${state === 'ready' ? 'live' : ''}`}>
          <span aria-hidden>{state === 'ready' ? '●' : state === 'loading' ? '○' : '✗'}</span>{' '}
          {state === 'ready' ? text.loaded : state === 'loading' ? text.loading : text.blocked}
        </span>
        <button aria-pressed={showSource} onClick={() => setShowSource(!showSource)}>
          {text.source}
        </button>
      </div>
      <div className={`workspace ${showSource ? 'split' : ''}`}>
        {showSource && (
          <label className="source">
            <span>{text.source}</span>
            <textarea readOnly spellCheck={false} value={snapshot.source} />
          </label>
        )}
        <div className="canvas">
          <div className="boundary">
            <span>{text.boundary}</span>
          </div>
          <div className="frame-host" ref={host} />
          {(state === 'navigation' || state === 'failed') && (
            <div className="notice" role="alert">
              <h2>{text.blocked}</h2>
              <p>{state === 'navigation' ? text.navigation : text.failed}</p>
              <p>{text.limit}</p>
            </div>
          )}
        </div>
      </div>
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
