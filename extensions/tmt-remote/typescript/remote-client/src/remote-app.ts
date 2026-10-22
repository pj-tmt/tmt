/**
 * Remote's one route (`/remote/…`, rows in `remote-routes.ts`). The markup of the connection check
 * and of the settings forms is in place from the start; the path decides which view shows, so a
 * switch between Remote's rows changes what the page shows and nothing else: no unmount, no
 * repaint of the shell or the rail, no second load. Each page's code starts the first time one of
 * its views shows, and ends with the route.
 */
import { remoteRegistryPage, type RemoteRegistryPage } from './remote-registry-page.js';
import { REMOTE_ITEMS, remoteItem, type RemoteItem } from './remote-routes.js';
import type { RemoteShell } from './remote-shell.js';

/** The two pages' code, loaded when a view of theirs first shows. */
export interface RemotePages {
  landing(lifetime: AbortSignal, registry: RemoteRegistryPage): Promise<void>;
  settings(shell: RemoteShell, registry: RemoteRegistryPage): Promise<void>;
}

export async function remoteApp(
  shell: RemoteShell,
  main: HTMLElement,
  bar: HTMLElement,
  /** Resolves once the settings views' stylesheet has loaded (they show only styled). */
  settingsStyles: () => Promise<void>,
  pages: () => Promise<RemotePages>,
): Promise<void> {
  const started = new Map<RemoteItem['page'], Promise<void>>();
  let registry: RemoteRegistryPage | undefined;
  let shown: RemoteItem | undefined;

  const start = (page: RemoteItem['page']): Promise<void> => {
    let run = started.get(page);
    if (!run) {
      run = (async () => {
        const code = await pages();
        if (shell.signal.aborted) return;
        registry ??= remoteRegistryPage(undefined, shell);
        if (page === 'landing') await code.landing(shell.signal, registry);
        else {
          // The settings views always talk to the door; the connection check waits for its own result.
          shell.describe();
          await code.settings(shell, registry);
        }
      })();
      started.set(page, run);
    }
    return run;
  };

  /** Show the view the path names. */
  const show = async (): Promise<void> => {
    const named = () => remoteItem(location.pathname) ?? REMOTE_ITEMS[0]!;
    if (named().page === 'settings') {
      await settingsStyles();
      // A later change of path is shown by its own call.
      if (shell.signal.aborted) return;
    }
    const item = named();
    const first = !shown;
    const changed = shown !== item;
    shown = item;
    document.documentElement.dataset.tmtPage = item.page;
    for (const page of main.querySelectorAll<HTMLElement>(':scope > [data-page]'))
      page.hidden = page.dataset.page !== item.page;
    for (const part of main.querySelectorAll<HTMLElement>('[data-view]'))
      part.hidden = !part.dataset.view!.split(' ').includes(item.id);
    const title = bar.querySelector('.header-title');
    if (title) title.textContent = item.title;
    shell.setTitle(item.title);
    if (changed && !first) window.scrollTo({ top: 0 });
    return start(item.page);
  };

  // The Connect view's one command, copied the way the connection check's are.
  const copy = document.getElementById('copy-connect-pair');
  const feedback = document.getElementById('copy-feedback-connect-pair');
  copy?.addEventListener(
    'click',
    async () => {
      try {
        await navigator.clipboard.writeText('tmt remote pair');
        if (feedback) feedback.textContent = 'Copied.';
      } catch {
        if (feedback)
          feedback.textContent = 'Copy failed. Select the command and copy it manually.';
      }
    },
    { signal: shell.signal },
  );

  // Back, forward, the rail and the menu all end in the path changing under this mounted route;
  // the shell announces it and the page follows it.
  window.addEventListener('popstate', () => void show().catch(() => {}), { signal: shell.signal });
  await show();
}
