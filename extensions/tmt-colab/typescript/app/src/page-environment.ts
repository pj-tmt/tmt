import { createContext, useContext } from 'react';
import { inShell } from './entry.js';
import { carriesThreadFragment } from './short-links.js';
import { text } from './strings.js';

/** What the document hands a page that is not its own to decide: the tab title, where focus goes
 * when a panel closes, which overlays are open, and the thread fragment of the address bar. The page
 * reads it instead of the browser's globals, so a page can be hosted somewhere other than its route
 * (#4192). The route's implementation is the default and behaves as the page always did. */
export interface PageEnvironment {
  /** Names the document for this page; the returned function gives the title back. */
  setTitle(title: string): () => void;
  /** A control that takes focus when a closing panel has nowhere better to return it. */
  focusFallback(): HTMLButtonElement | null;
  /** Whether a modal dialog is open, so a pointer outside the page's own popover is not the page's. */
  modalOpen(): boolean;
  /** Whether a menu or dialog other than the page's `except` panel is showing. */
  overlayOpen(except: string): boolean;
  /** The thread fragment of this location, or undefined where the location carries none. */
  threadFragment(): { read(): string | null; write(thread: string): void } | undefined;
}

/** The parts of the browser the route implementation reads. */
export interface PageEnvironmentHost {
  document: Pick<Document, 'querySelector' | 'querySelectorAll'> & { title: string };
  location: Pick<Location, 'pathname' | 'search' | 'hash'>;
  history: Pick<History, 'state' | 'replaceState'>;
  shell(): boolean;
}

export function routePageEnvironment(host: PageEnvironmentHost): PageEnvironment {
  return {
    setTitle(title) {
      host.document.title = title;
      return () => {
        host.document.title = text.product;
      };
    },
    focusFallback: () => host.document.querySelector<HTMLButtonElement>('.tmt-ui-header button'),
    modalOpen: () => host.document.querySelector('dialog:modal') !== null,
    overlayOpen: (except) =>
      [...host.document.querySelectorAll<HTMLElement>('[role="menu"], dialog[open]')].some(
        (node) => node.getClientRects().length > 0 && node.dataset.panel !== except,
      ),
    threadFragment() {
      const shell = host.shell();
      if (!carriesThreadFragment(host.location.pathname, shell)) return undefined;
      return {
        read: () => new URLSearchParams(host.location.hash.slice(1)).get('t'),
        write: (thread) =>
          host.history.replaceState(
            host.history.state,
            '',
            `${host.location.pathname}${shell ? host.location.search : ''}#t=${thread}`,
          ),
      };
    },
  };
}

const routeEnvironment = routePageEnvironment({
  get document() {
    return document;
  },
  get location() {
    return location;
  },
  get history() {
    return history;
  },
  shell: inShell,
});

const PageEnvironmentContext = createContext<PageEnvironment>(routeEnvironment);
export const PageEnvironmentProvider = PageEnvironmentContext.Provider;
export const usePageEnvironment = () => useContext(PageEnvironmentContext);
