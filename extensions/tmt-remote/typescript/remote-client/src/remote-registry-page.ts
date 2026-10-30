import { remotePath } from './remote-routes.js';
import { remoteShell, type RemoteShell } from './remote-shell.js';
import { remoteMenu } from './remote-menu.js';
import type { ViewerRole } from './remote-state.js';
import type { MemberSummary } from './members.js';
import { mountBrowserMenu, type BrowserMenuHost } from '@tmt/browser-ui/menu-host';
import { createIcon } from '@tmt/browser-ui/glyph';
import { browserUiClasses } from '@tmt/browser-ui/static';
import { createListTable, createPageSubheader } from '@tmt/browser-ui/list-table';
/** Remote pages' public-reference consumer. It never probes a foreign origin or shares a key. */
import {
  remoteRegistry,
  registryView,
  reference,
  RemoteRegistryError,
  type RemoteReference,
  type RemoteRegistryView,
  type ImportPlan,
} from './remote-registry.js';

function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = ''): HTMLElementTagNameMap[K] {
  const result = document.createElement(tag);
  result.textContent = text;
  return result;
}
function paintConnection(
  state: HTMLElement,
  connection: RemoteRegistryView['current']['connection'],
): void {
  state.dataset.tone =
    connection.text === 'Connected'
      ? 'working'
      : connection.text === 'Offline'
        ? 'blocked'
        : 'neutral';
  const mark = node('span');
  mark.className = browserUiClasses.stateMark;
  mark.dataset.shape = 'dot';
  mark.setAttribute('aria-hidden', 'true');
  const label = node('span', connection.text);
  label.className = browserUiClasses.chipLabel;
  state.replaceChildren(mark, label);
}
function action(text: string, run: () => void, variant?: 'destructive'): HTMLButtonElement {
  const button = node('button');
  const label = node('span', text);
  label.className = browserUiClasses.actionLabel;
  button.append(label);
  if (variant) button.dataset.variant = variant;
  button.type = 'button';
  button.className = 'tmt-ui-action';
  button.addEventListener('click', run);
  return button;
}
let fieldCount = 0;
function field(label: string, value: string): { label: HTMLElement; input: HTMLInputElement } {
  const wrapper = node('div');
  wrapper.className = browserUiClasses.field;
  const input = node('input');
  input.id = `remote-field-${++fieldCount}`;
  input.className = browserUiClasses.fieldControl;
  input.value = value;
  const text = node('label', label);
  text.className = browserUiClasses.fieldLabel;
  text.htmlFor = input.id;
  wrapper.append(text, input);
  return { label: wrapper, input };
}

function kindLabel(entry: RemoteReference): string {
  return entry.kind === 'local' ? 'Local' : 'Firebase';
}

/** The page's registry controller: observations in, and the reason a switch is held (settings). */
export type RemoteRegistryPage = ((
  state: string,
  address?: string,
  role?: ViewerRole | null,
) => void) & {
  /** Replaces the reason a foreign Open waits (the settings page holds unsent work). */
  blockedBy(reason: () => string): void;
  setSummary(summary?: MemberSummary): void;
  watchSummary(listener: (summary?: MemberSummary) => void): () => void;
  /** Browser-local catalog naming only; reuses the CAS dialog owner. */
  renameCurrent(opener: HTMLElement): void;
  /** Subscribe to this existing projection; never reads or grants authority. */
  watch(listener: (view: RemoteRegistryView) => void): () => void;
};

export function remoteRegistryPage(
  initialBlocked: () => string = () => '',
  shell?: RemoteShell,
): RemoteRegistryPage {
  let heldBy = initialBlocked;
  const blocked = (): string => heldBy();
  const root = document.getElementById('remotes');
  if (!root)
    return Object.assign(() => {}, {
      blockedBy: () => {},
      setSummary: () => {},
      watchSummary: () => () => {},
      renameCurrent: () => {},
      watch: () => () => {},
    });
  const listeners = new Set<(view: RemoteRegistryView) => void>();
  const summaryListeners = new Set<(summary?: MemberSummary) => void>();
  // Remove the entire untrusted fragment synchronously, before any admission can begin.
  const fragment = location.hash;
  if (fragment) history.replaceState(null, '', location.pathname + location.search);
  const oldList = document.getElementById('remote-list')!;
  const table = createListTable(
    [
      { id: 'icon', label: '', hideBelow: 'phone' },
      { id: 'name', label: 'Name' },
      { id: 'connection', label: 'Connection' },
      { id: 'role', label: 'Your role' },
      { id: 'members', label: 'Members', hideBelow: 'phone' },
      { id: 'active', label: 'Last active', hideBelow: 'phone' },
      { id: 'actions', label: '' },
    ],
    'My Remotes',
    'remote-registry-table',
  );
  const list = table.body;
  list.id = 'remote-list';
  oldList.replaceWith(table.root);
  const footer = node('p');
  footer.className = 'remote-list-footer';
  table.root.after(footer);
  const head = document.getElementById('remote-head')!;
  const actions = head.querySelector<HTMLElement>('.remote-head-actions')!;
  const primary = actions.querySelector('a[data-variant=primary]');
  if (primary) {
    primary.prepend(createIcon('plus'));
    actions.prepend(primary);
  }
  const subheader = createPageSubheader({
    title: 'My Remotes',
    subtitle: 'Names and addresses stay in this browser.',
    actions,
  });
  head.replaceChildren(...subheader.childNodes);
  head.className = subheader.className;
  const subtitle = head.querySelector<HTMLElement>('.tmt-ui-subheader-text p');
  const status = document.getElementById('remote-status')!;
  const joinHost = document.getElementById('remote-join')!;
  const joinMenu = mountBrowserMenu(joinHost, {
    label: 'Join a Remote',
    triggerText: 'Join',
    items: [
      { key: 'invite', label: 'Join with an invitation' },
      { key: 'address', label: 'Remember a Remote', separated: true },
    ],
    onSelect: (key) => {
      if (key === 'address') openAdd();
      else (shell ?? remoteShell())?.navigate(new URL(remotePath('join'), location.href).href);
    },
  });
  const join = joinHost.querySelector<HTMLButtonElement>('button')!;
  let rowMenus: Pick<BrowserMenuHost, 'dispose'>[] = [];
  (shell ?? remoteShell())?.signal.addEventListener(
    'abort',
    () => {
      joinMenu.dispose();
      rowMenus.forEach((item) => item.dispose());
    },
    { once: true },
  );
  let observed = 'Checking';
  let observedRole: ViewerRole | undefined;
  let summary: MemberSummary | undefined;
  let address: string | undefined =
    document.documentElement.dataset.tmtPage === 'settings'
      ? new URL('./', location.href).href
      : undefined;
  let currentDialog: HTMLDialogElement | undefined;
  (shell ?? remoteShell())?.signal.addEventListener(
    'abort',
    () => {
      const leaving = currentDialog;
      currentDialog = undefined;
      leaving?.close();
      leaving?.remove();
    },
    { once: true },
  );
  let catalog = { version: 1 as const, entries: [] as readonly RemoteReference[] };
  const menu = remoteMenu(
    {
      refresh: () => render(),
      recover: (action) => {
        const home = new URL(remotePath('overview'), location.href).href;
        if (action === 'Retry') {
          // The retry control of the view showing: the connection check, or the settings read.
          const retry = ['check', 'refresh']
            .map((id) => document.getElementById(id) as HTMLButtonElement | null)
            .find((button) => button && button.getClientRects().length > 0);
          if (retry && !retry.disabled) retry.click();
          else (shell ?? remoteShell())?.navigate(home);
        } else {
          if (location.pathname !== remotePath('overview'))
            (shell ?? remoteShell())?.navigate(home);
          // A retained draft may refuse navigation; do not focus its hidden destination.
          if (location.pathname !== remotePath('overview')) return;
          const pair = document.getElementById('copy-guide-pair');
          pair?.scrollIntoView();
          pair?.focus();
        }
      },
    },
    shell,
  );

  const showError = (error: unknown, slot: HTMLElement): void => {
    slot.textContent =
      error instanceof RemoteRegistryError ? error.message : 'The remotes could not be read';
  };
  const dialog = (
    title: string,
    opener: HTMLElement,
  ): {
    box: HTMLDialogElement;
    body: HTMLElement;
    buttons: HTMLElement;
    message: HTMLElement;
    end: () => void;
  } => {
    currentDialog?.close();
    // The document-level modal works from the global list or any current-door tab.
    const box = node('dialog');
    box.className = browserUiClasses.dialog;
    const header = node('header');
    header.className = browserUiClasses.dialogHeader;
    const heading = node('h2', title);
    heading.id = 'remote-dialog-heading';
    header.append(heading);
    box.setAttribute('aria-labelledby', heading.id);
    const content = node('div');
    content.className = browserUiClasses.dialogBody;
    const body = node('div');
    const message = node('p');
    message.setAttribute('role', 'status');
    content.append(body, message);
    const buttons = node('footer');
    buttons.className = browserUiClasses.dialogActions;
    box.append(header, content, buttons);
    // A registry action can originate in Settings while the global list is hidden.
    document.body.append(box);
    const end = (): void => box.close();
    box.addEventListener(
      'close',
      () => {
        box.remove();
        if (currentDialog === box) {
          currentDialog = undefined;
          (opener.isConnected ? opener : join).focus();
        }
      },
      { once: true },
    );
    currentDialog = box;
    return { box, body, buttons, message, end };
  };
  const save = async (
    run: () => Promise<void>,
    view: ReturnType<typeof dialog>,
  ): Promise<boolean> => {
    const buttons = [...view.box.querySelectorAll('button')];
    buttons.forEach((b) => {
      b.disabled = true;
    });
    try {
      await run();
      await render();
      view.end();
      return true;
    } catch (error) {
      showError(error, view.message);
      return false;
    } finally {
      buttons.forEach((b) => {
        b.disabled = false;
      });
    }
  };
  const importView = (plan: ImportPlan): void => {
    const view = dialog('Add these remotes to this browser?', join);
    view.body.append(
      node('p', 'Only names and addresses are copied. No keys, pairings or drafts.'),
    );
    const choices: (() => string | null)[] = [];
    for (const row of plan.rows) {
      const item = node('section');
      item.className = browserUiClasses.section;
      const name = node('h3', row.entry.name);
      name.className = browserUiClasses.sectionHeading;
      name.dataset.variant = 'title';
      item.append(name, node('p', `${kindLabel(row.entry)} · ${row.entry.url}`));
      if (row.status === 'duplicate') {
        item.append(node('p', `Already added as ${row.existingName}`));
        choices.push(() => null);
      } else if (row.status === 'collision') {
        item.append(node('p', `You already have a remote named ${row.entry.name}`));
        const alias = field('Remote name', row.suggestedName);
        const skip = node('input');
        skip.type = 'checkbox';
        skip.className = browserUiClasses.checkbox;
        const skipLabel = node('label', 'Skip');
        skipLabel.className = browserUiClasses.checkboxLabel;
        skipLabel.prepend(skip);
        item.append(alias.label, skipLabel);
        choices.push(() => (skip.checked ? null : alias.input.value));
      } else choices.push(() => row.entry.name);
      view.body.append(item);
    }
    const submit = action('Add', () => {
      void save(
        () =>
          remoteRegistry.confirmImport(
            plan,
            choices.map((choice) => choice()),
          ),
        view,
      );
    });
    const update = (): void => {
      submit.disabled = choices.every((choice) => choice() === null);
      view.message.textContent = submit.disabled ? 'Nothing new to add.' : '';
    };
    view.body.addEventListener('change', update);
    update();
    view.buttons.append(submit, action('Cancel', view.end));
    view.box.showModal();
  };
  const confirmAdd = (entry: RemoteReference, opener: HTMLElement): void => {
    const view = dialog(
      entry.kind === 'firestore'
        ? `Add Firebase remote ${entry.project}?`
        : `Add remote ${entry.name}?`,
      opener,
    );
    view.body.append(node('p', entry.kind === 'firestore' ? new URL(entry.url).host : entry.url));
    view.buttons.append(
      action('Add', () => {
        void save(() => remoteRegistry.add(entry.name, entry.url), view);
      }),
      action('Cancel', view.end),
    );
    view.box.showModal();
  };
  const rename = (entry: RemoteReference, opener: HTMLElement): void => {
    const view = dialog(`Rename ${entry.name}`, opener);
    const alias = field('Remote name', entry.name);
    view.body.append(alias.label);
    const submit = action('Save', () => {
      void save(() => remoteRegistry.rename(entry.name, alias.input.value), view);
    });
    submit.disabled = true;
    alias.input.addEventListener('input', () => {
      submit.disabled = alias.input.value === entry.name;
    });
    view.buttons.append(submit, action('Cancel', view.end));
    view.box.showModal();
  };
  const remove = (entry: RemoteReference, opener: HTMLElement): void => {
    const view = dialog(`Forget ${entry.name} in this browser?`, opener);
    view.body.append(
      node('p', 'This only removes the entry here. It does not sign out or revoke access.'),
    );
    view.buttons.append(
      action(
        'Forget',
        () => {
          void save(() => remoteRegistry.remove(entry.name), view).then((saved) => {
            if (!saved) return;
            status.replaceChildren(
              node('span', `Forgot ${entry.name} in this browser. Access is unchanged. `),
              action('Undo', () => {
                void remoteRegistry
                  .add(entry.name, entry.url)
                  .then(async () => {
                    await render();
                    status.textContent = `Restored ${entry.name}.`;
                  })
                  .catch((error: unknown) => showError(error, status));
              }),
            );
          });
        },
        'destructive',
      ),
      action('Cancel', view.end),
    );
    view.box.showModal();
  };
  const paint = (): void => {
    const projection = registryView(catalog, {
      pageUrl: location.href,
      state: observed,
      address,
      role: observedRole,
    });
    const visibleSummary =
      observed === 'Connected' && observedRole === 'owner' ? summary : undefined;
    summaryListeners.forEach((listener) => listener(visibleSummary));
    if (subtitle)
      subtitle.textContent = `${projection.rows.length} ${projection.rows.length === 1 ? 'remote' : 'remotes'} · Names and addresses stay in this browser.`;
    footer.replaceChildren(
      node(
        'span',
        `${projection.rows.length} ${projection.rows.length === 1 ? 'remote' : 'remotes'} in this view`,
      ),
      node('span', 'Saved in this browser · Forget leaves access unchanged'),
    );
    listeners.forEach((listener) => listener(projection));
    const existing = [...list.children] as HTMLElement[];
    if (
      existing.length === projection.rows.length &&
      existing.every((row, index) => {
        const entry = projection.rows[index]!;
        return (
          row.dataset.remote === entry.id &&
          row.dataset.url === entry.url &&
          row.dataset.thisMachine === String(entry.thisMachine)
        );
      })
    ) {
      for (const [index, row] of existing.entries()) {
        const entry = projection.rows[index]!;
        const role = row.querySelector<HTMLElement>('[data-role]')!;
        role.querySelector<HTMLElement>('.tmt-ui-chip-label')!.textContent = entry.roleLabel;
        role.dataset.roleTone =
          entry.role === 'owner' ? 'selected' : entry.role === undefined ? 'muted' : 'neutral';
        const state = row.querySelector<HTMLElement>('.tmt-ui-list-state')!;
        paintConnection(state, entry.connection);
        const countCell = row.querySelector<HTMLElement>('[data-summary=members]')!;
        const activeCell = row.querySelector<HTMLElement>('[data-summary=active]')!;
        countCell.textContent =
          entry.thisMachine && observedRole === 'owner' && summary?.memberCount != null
            ? String(summary.memberCount)
            : '';
        activeCell.textContent =
          entry.thisMachine && observedRole === 'owner' && summary?.lastActiveAtMs != null
            ? new Date(summary.lastActiveAtMs).toLocaleString()
            : '';
        const open = row.querySelector<HTMLAnchorElement>('[data-action="open"]')!;
        open.setAttribute('aria-disabled', String(!entry.thisMachine && !!blocked()));
        if (!entry.thisMachine && blocked()) open.removeAttribute('href');
        else
          open.href = entry.thisMachine
            ? new URL(remotePath('overview'), location.href).href
            : entry.url;
        open.title = !entry.thisMachine ? blocked() : '';
      }
      menu(projection);
      return;
    }
    rowMenus.forEach((item) => item.dispose());
    rowMenus = [];
    list.replaceChildren(
      ...projection.rows.map((entry) => {
        const leading = node('div');
        leading.className = browserUiClasses.listLeading;
        leading.append(createIcon(entry.backend === 'Local' ? 'terminal' : 'box'));
        const title = node('div');
        title.className = browserUiClasses.listTitle;
        const label = node('span');
        label.className = `${browserUiClasses.listLabel} remote-label`;
        const open = node('a', entry.name);
        open.href = entry.thisMachine
          ? new URL(remotePath('overview'), location.href).href
          : entry.url;
        open.dataset.action = 'open';
        open.setAttribute('role', 'link');
        open.setAttribute('aria-label', `Open ${entry.name}`);
        open.setAttribute('aria-disabled', String(!entry.thisMachine && !!blocked()));
        if (!entry.thisMachine && blocked()) open.removeAttribute('href');
        open.title = !entry.thisMachine ? blocked() : '';
        open.addEventListener('click', (event) => {
          event.preventDefault();
          if (entry.thisMachine) {
            (shell ?? remoteShell())?.navigate(new URL(remotePath('overview'), location.href).href);
            return;
          }
          void (async () => {
            try {
              if (!blocked() && entry.reference)
                location.assign(await remoteRegistry.navigation(entry.reference));
            } catch (error) {
              showError(error, status);
            }
          })();
        });
        label.append(open);
        const chip = (text: string) => {
          const box = node('span');
          box.className = browserUiClasses.chip;
          const value = node('span', text);
          value.className = browserUiClasses.chipLabel;
          box.append(value);
          return box;
        };
        if (entry.thisMachine && entry.name !== 'This machine') {
          const current = chip('This machine');
          current.className += ' remote-role';
          label.append(current);
        }
        const role = chip(entry.roleLabel);
        role.className += ' remote-role';
        role.dataset.role = '';
        role.dataset.roleTone =
          entry.role === 'owner' ? 'selected' : entry.role === undefined ? 'muted' : 'neutral';
        const description = node(
          'span',
          `${entry.backend}${entry.project ? ` · ${entry.project}` : ''} · ${entry.url}`,
        );
        description.className = browserUiClasses.listDescription;
        title.append(label, description);
        const state = node('div');
        state.className = `${browserUiClasses.listState} ${browserUiClasses.chip} remote-stage`;
        paintConnection(state, entry.connection);
        const actions = node('div');
        actions.className = browserUiClasses.listActions;
        if (entry.reference) {
          const reference = entry.reference;
          const destructive = node('span', 'Forget');
          destructive.className = 'remote-forget';
          const rowMenu = mountBrowserMenu(actions, {
            label: `Actions for ${entry.name}`,
            items: [
              { key: 'rename', label: 'Rename' },
              { key: 'forget', label: 'Forget', content: destructive, separated: true },
            ],
            onSelect: (key) => {
              const opener = actions.querySelector<HTMLButtonElement>('button')!;
              if (key === 'rename') rename(reference, opener);
              else remove(reference, opener);
            },
          });
          rowMenus.push(rowMenu);
        }
        const memberCount = node(
          'span',
          entry.thisMachine && observedRole === 'owner' && summary?.memberCount != null
            ? String(summary.memberCount)
            : '',
        );
        memberCount.dataset.summary = 'members';
        const lastActive = node(
          'span',
          entry.thisMachine && observedRole === 'owner' && summary?.lastActiveAtMs != null
            ? new Date(summary.lastActiveAtMs).toLocaleString()
            : '',
        );
        lastActive.dataset.summary = 'active';
        const row = table.row([leading, title, state, role, memberCount, lastActive, actions]);
        row.dataset.remote = entry.id;
        row.dataset.thisMachine = String(entry.thisMachine);
        row.dataset.url = entry.url;
        row.dataset.kind = entry.reference?.kind ?? 'local';
        return row;
      }),
    );
    menu(projection);
  };
  const render = async (): Promise<void> => {
    try {
      catalog = await remoteRegistry.list();
      paint();
    } catch (error) {
      showError(error, status);
    }
  };
  function openAdd(address = '', opener: HTMLElement = join): void {
    const view = dialog('Add remote', opener);
    const alias = field('Remote name', '');
    const url = field('Remote address or link', address);
    view.body.append(alias.label, url.label);
    view.buttons.append(
      action('Continue', () => {
        void reference(alias.input.value, url.input.value)
          .then((entry) => {
            // A validation that outlived its dialog is dropped: it never confirms or touches
            // a later opening.
            if (currentDialog !== view.box || !view.box.open || !view.box.isConnected) return;
            view.end();
            confirmAdd(entry, opener);
          })
          .catch((error: unknown) => showError(error, view.message));
      }),
      action('Cancel', view.end),
    );
    view.box.showModal();
  }
  void render().then(() => {
    if (fragment === '#add-remote') openAdd();
    else if (fragment === '#remotes') {
      root.scrollIntoView();
      join.focus({ preventScroll: true });
    }
  });
  const receive = (value: string): void => {
    void remoteRegistry
      .parseHandoff(value)
      .then(async (packet) => {
        const plan = await remoteRegistry.planImport(packet.catalog);
        importView(plan);
      })
      .catch((error: unknown) => showError(error, status));
  };
  if (fragment && !['#remotes', '#add-remote'].includes(fragment)) receive(fragment);
  window.addEventListener(
    'hashchange',
    () => {
      const value = location.hash;
      if (location.pathname === remotePath('join')) return;
      if (!value || ['#remotes', '#add-remote'].includes(value)) return;
      history.replaceState(null, '', location.pathname + location.search);
      receive(value);
    },
    { signal: (shell ?? remoteShell())?.signal },
  );
  const observe = (state: string, checkedAddress?: string, role?: ViewerRole | null): void => {
    observed = state;
    if (state !== 'Connected' || role === null) {
      observedRole = undefined;
      summary = undefined;
    } else if (role !== undefined) observedRole = role;
    if (checkedAddress) address = checkedAddress;
    paint();
  };
  return Object.assign(observe, {
    setSummary: (value?: MemberSummary): void => {
      summary = observed === 'Connected' ? value : undefined;
      paint();
    },
    watchSummary: (listener: (summary?: MemberSummary) => void): (() => void) => {
      summaryListeners.add(listener);
      listener(observed === 'Connected' && observedRole === 'owner' ? summary : undefined);
      return () => summaryListeners.delete(listener);
    },
    renameCurrent: (opener: HTMLElement): void => {
      const current = registryView(catalog, {
        pageUrl: location.href,
        state: observed,
        address,
        role: observedRole,
      }).current;
      if (current.reference) rename(current.reference, opener);
      else openAdd(current.url, opener);
    },
    watch: (listener: (view: RemoteRegistryView) => void): (() => void) => {
      listeners.add(listener);
      listener(
        registryView(catalog, {
          pageUrl: location.href,
          state: observed,
          address,
          role: observedRole,
        }),
      );
      return () => listeners.delete(listener);
    },
    blockedBy: (reason: () => string): void => {
      (shell ?? remoteShell())?.holdNavigation?.(reason);
      heldBy = reason;
    },
  });
}
