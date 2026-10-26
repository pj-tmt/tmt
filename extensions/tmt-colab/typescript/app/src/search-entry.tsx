/**
 * The global search's Colab half (#4841): the one quick switcher, mounted by a host that is not a
 * Colab route (the TMT shell on Ops, Digest, Remote or Settings). The host's bar owns the one
 * Command/Ctrl+K listener and calls `open`; the switcher adds none. Page data, project and author
 * filtering, ranking and navigation of a page stay Colab's; every other group comes from the
 * registry the host passes, as the snapshots its providers already hold.
 */
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import type { SearchRegistry } from '@tmt/browser-ui/search';
import { QuickSwitcherHost } from './quick-switcher-dialog.js';
import type { PageTransport } from './transport.js';

export interface SearchEntryOptions {
  /** The page list's read owner; the list is read again each time the dialog opens. */
  transport: PageTransport;
  space?: string;
  registry: SearchRegistry;
  /** The page the reader is on, which is never offered. */
  pageId?: string;
  navigate(pageId: string): Promise<unknown>;
  pageHref(pageId: string): string;
  /** Opens a route that is not a Colab page: a module, a section, a provider item or a project. */
  go(href: string): Promise<unknown>;
  /** The page list scoped to one project; without it no project is offered. */
  projectHref?(name: string): string;
}

export interface SearchEntry {
  /** Opens the dialog; closing it returns focus to `opener`. Inert after `dispose`. */
  open(opener: HTMLButtonElement): void;
  /** Closes the dialog and removes the React root. Later calls are inert. */
  dispose(): void;
}

export function mount(container: HTMLElement, options: SearchEntryOptions): SearchEntry {
  const root = createRoot(container);
  let disposed = false;
  let opener: ((button: HTMLButtonElement) => boolean) | undefined;
  let pending: HTMLButtonElement | undefined;
  const ready = (open: (button: HTMLButtonElement) => boolean) => {
    opener = open;
    if (pending) open(pending);
    pending = undefined;
  };
  root.render(
    <StrictMode>
      <QuickSwitcherHost
        transport={options.transport}
        {...(options.space === undefined ? {} : { space: options.space })}
        {...(options.pageId === undefined ? {} : { pageId: options.pageId })}
        routeKey="search"
        allowed
        navigate={options.navigate}
        pageHref={options.pageHref}
        search={{
          registry: options.registry,
          go: options.go,
          ...(options.projectHref ? { projectHref: options.projectHref } : {}),
        }}
        shortcut={false}
        onReady={ready}
      />
    </StrictMode>,
  );
  return {
    open(button) {
      if (disposed) return;
      if (opener) opener(button);
      else pending = button;
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      opener = undefined;
      pending = undefined;
      root.unmount();
    },
  };
}
