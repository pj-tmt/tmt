/** Fixed Remote rail children, projected from page-owned registry evidence.
 * Routes remain in `remote-routes.ts`; the shared rail owns presentation and selection. */
import type { ShellMenuItem } from '@tmt/browser-ui/shell';
import type { RemoteRegistryView } from './remote-registry.js';
import { remoteShell, type RemoteShell } from './remote-shell.js';
import { REMOTE_ITEMS, remoteItem, remotePath, type RemoteView } from './remote-routes.js';

interface Actions {
  /** Re-read the registry (another tab may have changed it); the page repaints the menu. */
  refresh(): Promise<void>;
  recover(action: string): void;
}
export function remoteMenu(actions: Actions, shell: RemoteShell | undefined = remoteShell()) {
  if (!shell) return () => {};
  const href = (view: RemoteView) => new URL(remotePath(view), location.href).href;
  let projection: RemoteRegistryView | undefined;
  function paint() {
    const view = projection?.current.connection;
    const showing = remoteItem(location.pathname)?.id;
    const item = (id: RemoteView): ShellMenuItem => {
      const row = REMOTE_ITEMS.find((candidate) => candidate.id === id)!;
      return { id, label: row.label, href: href(id), current: showing === id };
    };
    const recovery = view?.action;
    shell!.setMenu({
      title: 'Remote',
      groups: [
        {
          items: [
            {
              ...item('remotes'),
              current: ['remotes', 'create', 'join'].includes(showing ?? ''),
              total: projection?.rows.length,
            },
          ],
        },
        {
          label: 'This machine',
          items: [item('members'), item('my-devices')],
          action: recovery
            ? { icon: 'link', label: recovery, onActivate: () => actions.recover(recovery) }
            : undefined,
        },
      ],
      settings: {
        ...item('settings'),
        label: 'Remote settings',
        current: showing === 'settings' || showing === 'service',
      },
    });
  }

  paint();
  // Back, forward, the rail and a click on a row all end in the path changing under the same
  // mounted route; the shell announces it, and the menu follows it.
  window.addEventListener('popstate', paint, { signal: shell.signal });
  // The registry and the blocked reason change outside this menu: refresh when the pointer or
  // keyboard reaches it, as the old switcher did on opening.
  // The shell renders the menu element and may replace it, so the page listens on the document
  // until its route unmounts.
  const refresh = () => {
    paint();
    void actions.refresh();
  };
  const entered = (event: PointerEvent | FocusEvent) => {
    const menu = (event.target as Element | null)?.closest?.(
      '.tmt-ui-module-menu, .tmt-ui-rail-sub',
    );
    if (menu && !menu.contains(event.relatedTarget as Node | null)) refresh();
  };
  document.addEventListener('pointerover', entered, { signal: shell.signal });
  document.addEventListener('focusin', entered, { signal: shell.signal });
  return (next: RemoteRegistryView) => {
    projection = next;
    if (next.current.connection.word === 'connected' && next.address) shell.describe(next.address);
    paint();
  };
}
