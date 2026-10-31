/** Root-owned global search lifetime. Colab owns its transport and dialog; the leaf owns keys. */
import {
  createSearchRegistry,
  type SearchItem,
  type SearchProvider,
  type SearchRegistry,
  type SearchSnapshot,
} from '@tmt/browser-ui/search';
import type { ShellAnswer, ShellItem } from '@tmt/browser-ui/shell';

export interface SearchSession {
  open(opener: HTMLButtonElement): void;
  dispose(): void;
}

/** The one adapter to the Colab-owned #5267 export. No transport protocol lives in this root. */
export interface SearchModule {
  mountSearchSession(
    container: HTMLElement,
    options: {
      registry: SearchRegistry;
      navigate(pageId: string): Promise<unknown>;
      pageHref(pageId: string): string;
      go(href: string): Promise<unknown>;
      projectHref(name: string): string;
      placeholder: string;
    },
  ): SearchSession;
}

/** Same described routes as the rail. Unavailable modules and cross-origin URLs contribute none. */
export function navigationSnapshot(
  answer: ShellAnswer | undefined,
  rail: readonly ShellItem[],
  here: string,
  href: (path: string) => string,
): SearchSnapshot {
  if (!answer) return { state: 'unavailable' };
  const origin = new URL(here).origin;
  const items: SearchItem[] = [];
  const seen = new Set<string>();
  const add = (key: string, label: string, path: string, icon: string, detail?: string) => {
    let url: URL;
    try {
      url = new URL(path, here);
    } catch {
      return;
    }
    if (url.origin !== origin || url.username || url.password || seen.has(key)) return;
    seen.add(key);
    items.push({ key, label, href: url.pathname + url.search + url.hash, icon, detail });
  };
  for (const item of rail) {
    if (item.disabled) continue;
    const module = answer.modules.find(
      (module) => module.extension === item.id && !('module' in module),
    );
    // Home and TMT settings are root entries; every other item needs a live description.
    if (!module && item.id !== 'home' && item.id !== 'settings') continue;
    add(item.id, item.label, item.href, item.icon);
    if (!module) continue;
    for (const section of [...module.menu, ...module.settings])
      add(
        `${module.extension}:${section.id}`,
        section.label,
        href(section.path),
        item.icon,
        item.label,
      );
  }
  return { state: 'ready', items };
}

export interface RootSearchOptions {
  load(entry: string): Promise<SearchModule>;
  container(): HTMLElement;
  failed(message: string): void;
  navigate(href: string): void;
}

/**
 * A route owns its pending open and Colab session. The module import is cached across routes;
 * registry registrations and transport never outlive the route/session context that admitted them.
 */
export class RootSearch {
  private imports = new Map<string, Promise<SearchModule>>();
  private lifetime = 0;
  private request = 0;
  private closed = false;
  private entry?: string;
  private session?: SearchSession;
  private host?: HTMLElement;
  private registry = createSearchRegistry();
  private snapshot: SearchSnapshot = { state: 'unavailable' };
  private listeners = new Set<() => void>();

  constructor(private readonly options: RootSearchOptions) {
    this.registerNavigation();
  }

  private registerNavigation() {
    const provider: SearchProvider = {
      key: 'navigation',
      label: 'Navigation',
      getSnapshot: () => this.snapshot,
      subscribe: (listener) => {
        this.listeners.add(listener);
        return () => void this.listeners.delete(listener);
      },
    };
    this.registry.register(provider);
  }

  /** Existing adapters register only snapshots they already hold, on this context's registry. */
  register(provider: SearchProvider): () => void {
    return this.registry.register(provider);
  }

  private clear() {
    ++this.lifetime;
    ++this.request;
    this.session?.dispose();
    this.session = undefined;
    this.host?.remove();
    this.host = undefined;
    this.registry.dispose();
    this.listeners.clear();
    this.registry = createSearchRegistry();
    this.registerNavigation();
  }

  /** Called for route or verified session/context replacement, never to acquire a session. */
  replace() {
    if (this.closed) return;
    this.entry = undefined;
    this.snapshot = { state: 'unavailable' };
    this.clear();
  }

  update(entry: string | undefined, snapshot: SearchSnapshot) {
    if (this.closed) return;
    if (entry !== this.entry) this.clear();
    this.entry = entry;
    this.snapshot = snapshot;
    for (const listener of this.listeners) listener();
    // Prefetch only imports a confined, described module; Colab admission waits for open.
    if (entry) void this.prefetch(entry).catch(() => {});
  }

  private prefetch(entry: string): Promise<SearchModule> {
    let pending = this.imports.get(entry);
    if (!pending) {
      pending = this.options.load(entry);
      this.imports.set(entry, pending);
    }
    return pending;
  }

  open = async (opener: HTMLButtonElement): Promise<void> => {
    if (this.closed) return;
    const mine = ++this.request;
    const lifetime = this.lifetime;
    const current = () => !this.closed && lifetime === this.lifetime && mine === this.request;
    if (!this.entry) {
      this.options.failed('Search is not available until this door describes Colab.');
      return;
    }
    try {
      const module = await this.prefetch(this.entry);
      if (!current() || !opener.isConnected) return;
      if (!this.session) {
        this.host = this.options.container();
        const pageHref = (id: string) => `/colab/pages/${encodeURIComponent(id)}`;
        const go = async (href: string) => this.options.navigate(href);
        this.session = module.mountSearchSession(this.host, {
          registry: this.registry,
          navigate: (id) => go(pageHref(id)),
          pageHref,
          go,
          projectHref: (name) => `/colab/?scope=${encodeURIComponent(name)}`,
          placeholder: 'Search TMT…',
        });
      }
      if (current()) this.session.open(opener);
    } catch {
      if (!current()) return;
      this.session?.dispose();
      this.session = undefined;
      this.host?.remove();
      this.host = undefined;
      this.options.failed('Search could not load. Reload this page to try again.');
      if (opener.isConnected) opener.focus();
    }
  };

  dispose() {
    if (this.closed) return;
    this.clear();
    this.registry.dispose();
    this.closed = true;
    this.imports.clear();
  }
}
