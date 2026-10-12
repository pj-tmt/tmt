/** Remote's static BrowserMenu adapter; navigation and observations remain page-owned. */
import type { RemoteCatalog, RemoteReference } from './remote-registry.js';
import { remoteState, currentRemote } from './remote-state.js';

interface Actions {
  refresh(): Promise<void>;
  open(entry: RemoteReference): void;
  add(): void;
  manage(): void;
  recover(action: string): void;
  blocked(): string;
}
export function remoteMenu(actions: Actions) {
  const host = document.getElementById('remote-switcher');
  if (!host) return () => {};
  const trigger = document.createElement('button');
  trigger.type = 'button';
  trigger.className = 'tmt-ui-menu-trigger remote-chip';
  trigger.setAttribute('aria-haspopup', 'menu');
  trigger.setAttribute('aria-controls', 'remote-menu');
  trigger.setAttribute('aria-expanded', 'false');
  const surface = document.createElement('div');
  surface.id = 'remote-menu';
  surface.popover = 'auto';
  surface.className = 'tmt-ui-menu-surface remote-menu';
  surface.setAttribute('role', 'menu');
  surface.setAttribute('aria-label', 'Remotes');
  const status = document.createElement('p');
  status.className = 'tmt-ui-menu-reason';
  host.append(trigger, surface);
  let catalog: RemoteCatalog = { version: 1, entries: [] };
  let state = 'Checking';
  let address = '';
  let search = '';
  let searchAt = 0;
  const buttons = new Map<string, HTMLButtonElement>();
  const selected = () =>
    catalog.entries.find((entry) => currentRemote(entry, address, location.href));
  const enabled = () => [...surface.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
  function close(refocus = false) {
    if (surface.matches(':popover-open')) surface.hidePopover();
    if (refocus) trigger.focus({ preventScroll: true });
  }
  function item(key: string, label: string, run: () => void) {
    let button = buttons.get(key);
    if (!button) {
      button = document.createElement('button');
      button.type = 'button';
      button.className = 'tmt-ui-menu-item';
      button.tabIndex = -1;
      button.dataset.key = key;
      buttons.set(key, button);
    }
    button.onclick = () => {
      close();
      run();
    };
    button.dataset.search = label;
    button.textContent = label;
    return button;
  }
  function paint() {
    const view = remoteState(state);
    const alias = selected()?.name ?? 'this machine';
    trigger.textContent = `${view.mark} ${alias} ▾`;
    trigger.title = `Remote: ${alias}, ${view.word}`;
    trigger.setAttribute('aria-label', trigger.title);
    trigger.dataset.tone = view.tone;
    const entries = [...catalog.entries].sort(
      (a, b) => Number(a.kind === 'firestore') - Number(b.kind === 'firestore'),
    );
    const keep = new Set<string>();
    const reason = actions.blocked();
    const append = (
      key: string,
      label: string,
      run: () => void,
      checked: boolean,
      disabled: boolean,
    ) => {
      const button = item(key, label, run);
      button.dataset.search = key.startsWith('remote:') ? key.slice(7) : 'this machine';
      button.setAttribute('role', 'menuitemradio');
      button.setAttribute('aria-checked', String(checked));
      if (checked) {
        const check = document.createElement('span');
        check.textContent = '✓';
        check.setAttribute('aria-hidden', 'true');
        button.append(check);
      }
      button.disabled = disabled;
      button.dataset.tone = checked ? view.tone : 'muted';
      button.setAttribute('aria-describedby', 'remote-menu-reason');
      keep.add(key);
      // Moving only on order change keeps focus on the same DOM node during observations.
      return button;
    };
    const rows: HTMLButtonElement[] = [];
    if (!selected())
      rows.push(
        append(
          'current',
          `${view.mark} this machine · ${view.word}`,
          () => {},
          true,
          !!view.action,
        ),
      );
    for (const entry of entries) {
      const here = currentRemote(entry, address, location.href),
        shown = remoteState(state, here);
      rows.push(
        append(
          `remote:${entry.name}`,
          here
            ? `${shown.mark} ${entry.name} · ${shown.word}`
            : `Open ${entry.name} · ${entry.kind === 'local' ? 'Local' : `Firebase · ${entry.project}`} · not checked`,
          () => {
            if (!here) actions.open(entry);
          },
          here,
          here ? !!shown.action : !!reason,
        ),
      );
    }
    if (view.action) {
      const key = 'recover';
      keep.add(key);
      const button = item(key, view.action, () => actions.recover(remoteState(state).action));
      button.setAttribute('role', 'menuitem');
      rows.push(button);
    }
    for (const [key, label, run] of [
      ['add', 'Add remote…', actions.add],
      ['manage', 'Manage remotes', actions.manage],
    ] as const) {
      keep.add(key);
      const button = item(key, label, run);
      button.setAttribute('role', 'menuitem');
      rows.push(button);
    }
    status.id = 'remote-menu-reason';
    status.textContent = reason || (view.action ? view.text : '');
    status.hidden = !status.textContent;
    for (const [key, button] of buttons)
      if (!keep.has(key)) {
        button.remove();
        buttons.delete(key);
      }
    rows.forEach((button, index) => {
      if (surface.children[index] !== button)
        surface.insertBefore(button, surface.children[index] ?? null);
    });
    surface.append(status);
    const focused = enabled().find((button) => button === document.activeElement) ?? enabled()[0];
    for (const button of buttons.values()) button.tabIndex = button === focused ? 0 : -1;
  }
  function place() {
    const anchor = trigger.getBoundingClientRect();
    const box = surface.getBoundingClientRect();
    const width = document.documentElement.clientWidth;
    surface.style.left = `${Math.max(8, Math.min(anchor.left, width - box.width - 8))}px`;
    const below = innerHeight - anchor.bottom - 8;
    const above = anchor.top - 8;
    const up = box.height > below && above > below;
    surface.style.maxHeight = `${Math.max(0, up ? above : below)}px`;
    surface.style.top = `${up ? Math.max(8, anchor.top - Math.min(box.height, above)) : anchor.bottom}px`;
  }
  async function open(end = false) {
    await actions.refresh();
    if (document.activeElement !== trigger) return;
    paint();
    if (!surface.matches(':popover-open')) surface.showPopover();
    place();
    search = '';
    const first = end ? enabled().at(-1) : enabled()[0];
    first?.focus();
  }
  trigger.addEventListener('click', () => {
    if (surface.matches(':popover-open')) close();
    else {
      trigger.focus({ preventScroll: true });
      void open();
    }
  });
  trigger.addEventListener('keydown', (event) => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      void open(event.key === 'ArrowUp');
    }
  });
  surface.addEventListener('toggle', () => {
    trigger.setAttribute('aria-expanded', String(surface.matches(':popover-open')));
    if (surface.matches(':popover-open')) place();
  });
  surface.addEventListener('focusin', () => {
    for (const button of buttons.values())
      button.tabIndex = button === document.activeElement ? 0 : -1;
  });
  surface.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      close(true);
      return;
    }
    if (event.key === 'Tab') {
      close();
      return;
    }
    if (event.isComposing || event.altKey || event.ctrlKey || event.metaKey) return;
    const items = enabled(),
      at = items.findIndex((item) => item === document.activeElement);
    let next: HTMLButtonElement | undefined;
    if (event.key === 'ArrowDown') next = items[(at + 1) % items.length];
    else if (event.key === 'ArrowUp') next = items[(at - 1 + items.length) % items.length];
    else if (event.key === 'Home') next = items[0];
    else if (event.key === 'End') next = items.at(-1);
    else if (event.key.length === 1 && event.key !== ' ') {
      const now = performance.now();
      search = (now - searchAt > 700 ? '' : search) + event.key.toLowerCase();
      searchAt = now;
      next = [...items.slice(at + 1), ...items.slice(0, at + 1)].find((item) =>
        (item.dataset.search ?? '').toLowerCase().startsWith(search),
      );
    } else return;
    event.preventDefault();
    next?.focus();
  });
  window.addEventListener('resize', () => {
    if (surface.matches(':popover-open')) place();
  });
  window.addEventListener(
    'scroll',
    () => {
      if (surface.matches(':popover-open')) place();
    },
    true,
  );
  return (next: RemoteCatalog, observed: string, checkedAddress: string) => {
    catalog = next;
    state = observed;
    address = checkedAddress;
    paint();
  };
}
