import { describe, expect, it } from 'vite-plus/test';
import { routePageEnvironment, type PageEnvironmentHost } from '../src/page-environment.js';

const page = '12345678-0000-4000-8000-000000000001';

function host(over: {
  pathname: string;
  hash?: string;
  shell: boolean;
  modal?: boolean;
  overlays?: { panel?: string; visible: boolean }[];
}) {
  const calls: unknown[][] = [];
  const header = { id: 'header-button' };
  const environment: PageEnvironmentHost = {
    document: {
      title: 'before',
      querySelector: ((selector: string) =>
        selector === 'dialog:modal'
          ? over.modal
            ? {}
            : null
          : selector === '.tmt-ui-header button'
            ? header
            : null) as Document['querySelector'],
      querySelectorAll: (() =>
        (over.overlays ?? []).map((overlay) => ({
          dataset: { panel: overlay.panel },
          getClientRects: () => (overlay.visible ? [{}] : []),
        }))) as unknown as Document['querySelectorAll'],
    },
    location: { pathname: over.pathname, search: '?read=1', hash: over.hash ?? '' },
    history: {
      state: 'kept',
      replaceState: (...args: unknown[]) => void calls.push(args),
    },
    shell: () => over.shell,
  };
  return { environment, calls, header };
}

describe('the route page environment', () => {
  it('titles the tab for the page and gives the product title back', () => {
    const { environment } = host({ pathname: '/', shell: true });
    const route = routePageEnvironment(environment);
    const release = route.setTitle('A page · Colab');
    expect(environment.document.title).toBe('A page · Colab');
    release();
    expect(environment.document.title).toBe('Colab');
  });

  it('names the header button as the focus fallback and sees a modal dialog', () => {
    const open = host({ pathname: '/', shell: true, modal: true });
    expect(routePageEnvironment(open.environment).focusFallback()).toBe(open.header);
    expect(routePageEnvironment(open.environment).modalOpen()).toBe(true);
    expect(routePageEnvironment(host({ pathname: '/', shell: true }).environment).modalOpen()).toBe(
      false,
    );
  });

  it('counts a visible menu or dialog unless it is the page’s own panel', () => {
    const { environment } = host({
      pathname: '/',
      shell: true,
      overlays: [
        { panel: 'chat', visible: true },
        { panel: 'about', visible: false },
      ],
    });
    const route = routePageEnvironment(environment);
    expect(route.overlayOpen('chat')).toBe(false);
    expect(route.overlayOpen('about')).toBe(true);
  });

  it('carries a thread in the page route of the app and keeps its search', () => {
    const { environment, calls } = host({
      pathname: `/colab/pages/${page}`,
      hash: '#t=thread-1',
      shell: true,
    });
    const fragment = routePageEnvironment(environment).threadFragment();
    expect(fragment?.read()).toBe('thread-1');
    fragment?.write('thread-2');
    expect(calls).toEqual([['kept', '', `/colab/pages/${page}?read=1#t=thread-2`]]);
  });

  it('carries a thread in the short location of a page’s own origin, without its search', () => {
    const { environment, calls } = host({ pathname: '/p/12345678', shell: false });
    routePageEnvironment(environment).threadFragment()?.write('thread-3');
    expect(calls).toEqual([['kept', '', '/p/12345678#t=thread-3']]);
  });

  it('carries no thread where the location is neither', () => {
    expect(
      routePageEnvironment(host({ pathname: '/colab/', shell: true }).environment).threadFragment(),
    ).toBeUndefined();
    expect(
      routePageEnvironment(host({ pathname: '/', shell: false }).environment).threadFragment(),
    ).toBeUndefined();
  });
});
