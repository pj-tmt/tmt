import {
  loadShellAnswer,
  mountShell,
  rememberedShellAnswer,
  shellAddress,
  shellRail,
  type ShellAnswer,
  type ShellControls,
  type ShellMenu,
  type ShellModule,
} from './shell.js';

/**
 * The one TMT web app (#3937, spec c5ccd17a): one document, one `mountShell` instance and one
 * History-API router for every TMT route. Each route is a module that mounts into the shell's
 * main area and header slot through a `ShellSlot` and gives them back when the route changes.
 */

/** What a route module receives: its part of the one shell. */
export interface ShellSlot {
  /** The content area; the module owns its children until it unmounts. */
  readonly main: HTMLElement;
  /** The header slot (`.tmt-ui-shell-bar`); the module renders its one header here. */
  readonly bar: HTMLElement;
  /** The route's path prefix, such as `/colab/`; `/` for Home. */
  readonly basePath: string;
  /** The composed shell this page read, if any (display data only). */
  answer(): ShellAnswer | undefined;
  /** The module menu; `select` may take over a plain click on one of its items. */
  setMenu(menu: ShellMenu | undefined, select?: (id: string, href: string) => boolean): void;
  /** The document title; ` · TMT` is appended. */
  setTitle(title: string): void;
  /** Go to a TMT URL without reloading; another module's route unmounts this one. */
  navigate(href: string, options?: { replace?: boolean }): void;
  /** Read the composed shell now, for a route that waits (Remote's landing waits for pairing);
   * `address` is a mount address the page already verified. */
  describe(address?: string): void;
  /** Called once the read settles: the answer, or undefined without an owner session. */
  described(listener: (answer: ShellAnswer | undefined) => void): void;
}

/** A lazily loaded route module. `mount` may return the function that releases everything it
 * started (sockets, timers, listeners, React roots); the host calls it before the next module. */
export interface ShellRouteModule {
  mount(slot: ShellSlot): void | (() => void) | Promise<void | (() => void)>;
}

export interface ShellRoute {
  id: string;
  /** Path prefixes this route owns, such as `['/colab', '/colab/']`; `'/'` alone matches Home only. */
  paths: readonly string[];
  /** The rail item this route lights: a module's extension, or `home` / `settings`. */
  rail: string;
  /** Leave the composed shell unread until the module calls `describe` (unpaired pages). */
  quiet?: boolean;
  /** `settled` resolves with the composed shell once it is read (or undefined without one). */
  load(
    answer: ShellAnswer | undefined,
    settled: () => Promise<ShellAnswer | undefined>,
  ): Promise<ShellRouteModule>;
}

export interface ShellHost {
  navigate(href: string, options?: { replace?: boolean }): void;
  /** The route that owns `pathname`, if any. */
  route(pathname: string): ShellRoute | undefined;
  dispose(): void;
}

export function matchRoute(routes: readonly ShellRoute[], pathname: string) {
  let best: ShellRoute | undefined;
  let length = -1;
  for (const route of routes)
    for (const path of route.paths) {
      const hit =
        path === '/'
          ? pathname === '/'
          : pathname === path || (path.endsWith('/') && pathname.startsWith(path));
      if (hit && path.length > length) {
        best = route;
        length = path.length;
      }
    }
  return best;
}

/** The rail as this app shows it: `route`'s module lit, every item linking to the app's own route
 * for it rather than to a module's mount. */
export function appRail(
  answer: ShellAnswer | undefined,
  routes: readonly ShellRoute[],
  place: { origin: string; href: string },
  route?: ShellRoute,
) {
  const host: ShellModule | undefined =
    route && route.rail !== 'home' && route.rail !== 'settings'
      ? {
          extension: route.rail,
          rail: {
            label: route.rail,
            icon: 'box',
            path: `${place.origin}${route.paths.at(-1)}`,
            order: 0,
          },
          menu: [],
          settings: [],
        }
      : undefined;
  const rail = shellRail(answer, place, host);
  for (const item of [...rail.main, ...rail.bottom]) {
    const owner = routes.find((candidate) => candidate.rail === item.id);
    if (owner && item.id !== 'home') item.href = `${place.origin}${owner.paths.at(-1)}`;
  }
  return rail;
}

/**
 * Start the one app in `root`, a `.tmt-ui-shell` element holding `.tmt-ui-shell-content` with
 * `.tmt-ui-shell-bar` and `.tmt-ui-shell-main`. A route the host does not own is a full load.
 */
export function startShellHost(
  root: HTMLElement,
  routes: readonly ShellRoute[],
  options: { failed?(main: HTMLElement, route: ShellRoute, error: unknown): void } = {},
): ShellHost {
  const document = root.ownerDocument;
  const window = document.defaultView!;
  const bar = root.querySelector<HTMLElement>(
    ':scope > .tmt-ui-shell-content > .tmt-ui-shell-bar',
  )!;
  const main = root.querySelector<HTMLElement>(
    ':scope > .tmt-ui-shell-content > .tmt-ui-shell-main',
  )!;
  let answer: ShellAnswer | undefined = rememberedShellAnswer();
  let current: { route: ShellRoute; release?: () => void; generation: number } | undefined;
  let menu: ShellMenu | undefined;
  let select: ((id: string, href: string) => boolean) | undefined;
  let asked = false;
  let settled = false;
  const listeners: ((answer: ShellAnswer | undefined) => void)[] = [];
  let generation = 0;

  const place = () => ({
    origin: window.location.origin,
    href: window.location.href.split(/[?#]/)[0]!,
  });
  /** The rail links to this app's routes, not to a module's own mount. */
  const view = () => ({ rail: appRail(answer, routes, place(), current?.route), menu });
  const controls: ShellControls = mountShell(root, view(), (id, href) => {
    if (select?.(id, href)) return true;
    return go(href);
  });
  const paint = () => controls.update(view());

  const describe = (known?: string) => {
    if (asked) return;
    asked = true;
    void (known === undefined ? shellAddress() : Promise.resolve(known)).then(async (address) => {
      const read = address === undefined ? undefined : await loadShellAnswer(address);
      if (read) {
        answer = read;
        paint();
      }
      settled = true;
      for (const listener of listeners.splice(0)) listener(read);
    });
  };

  const slot = (route: ShellRoute, mine: number): ShellSlot => ({
    main,
    bar,
    basePath: route.paths.at(-1)!,
    answer: () => answer,
    setMenu(next, handler) {
      if (current?.generation !== mine) return;
      menu = next;
      select = handler;
      paint();
    },
    setTitle(title) {
      if (current?.generation === mine) document.title = `${title} · TMT`;
    },
    navigate: (href, options) => void go(href, options),
    describe,
    described(listener) {
      if (settled) listener(answer);
      else listeners.push(listener);
    },
  });

  async function show() {
    const route = matchRoute(routes, window.location.pathname);
    if (!route) return window.location.reload();
    if (current?.route === route) return paint();
    current?.release?.();
    const mine = ++generation;
    current = { route, generation: mine };
    menu = undefined;
    select = undefined;
    bar.replaceChildren();
    main.replaceChildren();
    main.dataset.route = route.id;
    paint();
    if (!route.quiet) describe();
    try {
      const module = await route.load(
        answer,
        () => new Promise((resolve) => (settled ? resolve(answer) : listeners.push(resolve))),
      );
      if (current.generation !== mine) return;
      const release = await module.mount(slot(route, mine));
      if (current.generation !== mine) return release?.();
      current.release = release ?? undefined;
    } catch (error) {
      if (current.generation !== mine) return;
      main.replaceChildren();
      options.failed?.(main, route, error);
    }
  }

  /** Same-origin URL this app owns: change the history entry and show its route. */
  function go(href: string, { replace = false } = {}): boolean {
    const url = new URL(href, window.location.href);
    if (url.origin !== window.location.origin || !matchRoute(routes, url.pathname)) return false;
    // A fragment on the same page is the page's own (a module section): the browser moves it and
    // fires hashchange, as for any in-page link.
    const here = new URL(window.location.href);
    if (url.hash && url.pathname === here.pathname && url.search === here.search) {
      if (url.hash !== here.hash) window.location.hash = url.hash;
      return true;
    }
    if (url.href !== window.location.href)
      window.history[replace ? 'replaceState' : 'pushState'](null, '', url.href);
    // A module's own router (Colab's) follows history through popstate, as on Back.
    if (matchRoute(routes, url.pathname) === current?.route)
      window.dispatchEvent(new PopStateEvent('popstate'));
    else void show();
    return true;
  }

  // Links anywhere in the page that point at another of this app's routes stay in the app.
  // A module's own in-module links (Colab's router) handle the click first.
  const click = (event: MouseEvent) => {
    if (event.defaultPrevented || event.button || event.metaKey || event.ctrlKey) return;
    if (event.shiftKey || event.altKey) return;
    const anchor = (event.target as Element | null)?.closest?.('a[href]');
    if (
      !(anchor instanceof window.HTMLAnchorElement) ||
      anchor.target ||
      anchor.hasAttribute('download')
    )
      return;
    const url = new URL(anchor.href);
    const route = matchRoute(routes, url.pathname);
    if (url.origin !== window.location.origin || !route) return;
    if (route === current?.route && url.pathname === window.location.pathname) return;
    if (route === current?.route) return;
    event.preventDefault();
    go(url.href);
  };
  const popstate = () => void show();
  document.addEventListener('click', click);
  window.addEventListener('popstate', popstate);
  void show();
  return {
    navigate: (href, options) => void go(href, options),
    route: (pathname) => matchRoute(routes, pathname),
    dispose() {
      document.removeEventListener('click', click);
      window.removeEventListener('popstate', popstate);
      current?.release?.();
      controls.dispose();
    },
  };
}
