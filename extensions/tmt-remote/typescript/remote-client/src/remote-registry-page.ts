import { remoteShell, type RemoteShell } from './remote-shell.js';
import { remoteMenu } from './remote-menu.js';
import type { ViewerRole } from './remote-state.js';
import { mountBrowserMenu, type BrowserMenuHost } from '@tmt/browser-ui/menu-host';
import { createIcon } from '@tmt/browser-ui/glyph';
import { browserUiClasses } from '@tmt/browser-ui/static';
/** Remote pages' public-reference consumer. It never probes a foreign origin or shares a key. */
import {
  remoteRegistry,
  registryView,
  reference,
  RemoteRegistryError,
  type RemoteReference,
  type ImportPlan,
} from './remote-registry.js';

function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = ''): HTMLElementTagNameMap[K] {
  const result = document.createElement(tag);
  result.textContent = text;
  return result;
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
};

export function remoteRegistryPage(
  initialBlocked: () => string = () => '',
  shell?: RemoteShell,
): RemoteRegistryPage {
  let heldBy = initialBlocked;
  const blocked = (): string => heldBy();
  const root = document.getElementById('remotes');
  if (!root) return Object.assign(() => {}, { blockedBy: () => {} });
  // Remove the entire untrusted fragment synchronously, before any admission can begin.
  const fragment = location.hash;
  if (fragment) history.replaceState(null, '', location.pathname + location.search);
  const list = document.getElementById('remote-list')!;
  const status = document.getElementById('remote-status')!;
  const joinHost = document.getElementById('remote-join')!;
  const joinMenu = mountBrowserMenu(joinHost, {
    label: 'Join a Remote',
    triggerText: 'Join a Remote',
    items: [
      { key: 'invite', label: 'Join with an invitation' },
      { key: 'address', label: 'Remember a Remote', separated: true },
    ],
    onSelect: (key) => {
      if (key === 'address') openAdd();
      else
        (shell ?? remoteShell())?.navigate(new URL('connect', new URL('./', location.href)).href);
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
  let address: string | undefined =
    document.documentElement.dataset.tmtPage === 'settings'
      ? new URL('./', location.href).href
      : undefined;
  let currentDialog: HTMLDialogElement | undefined;
  let catalog = { version: 1 as const, entries: [] as readonly RemoteReference[] };
  const settings = new URL('settings', new URL('./', location.href));
  const menu = remoteMenu(
    {
      refresh: () => render(),
      recover: (action) => {
        const home = new URL('./', location.href).href;
        if (action === 'Retry') {
          // The retry control of the view showing: the connection check, or the settings read.
          const retry = ['check', 'refresh']
            .map((id) => document.getElementById(id) as HTMLButtonElement | null)
            .find((button) => button && button.getClientRects().length > 0);
          if (retry && !retry.disabled) retry.click();
          else (shell ?? remoteShell())?.navigate(home);
        } else if (document.documentElement.dataset.tmtPage === 'settings')
          (shell ?? remoteShell())?.navigate(home);
        else {
          const pair = [
            'copy-refused-pair',
            'copy-missing-pair',
            'copy-unreadable-pair',
            'copy-different-pair',
          ]
            .map((id) => document.getElementById(id))
            .find((button) => button && button.getClientRects().length > 0);
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
    // The registry is the settings view's: a handoff that arrives on another view (a link to the
    // landing) asks its questions where the remotes are.
    if (document.documentElement.dataset.tmtPage !== 'settings')
      (shell ?? remoteShell())?.navigate(settings.href);
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
    root.append(box);
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
          entry.roleLabel === 'Admin'
            ? 'selected'
            : entry.roleLabel === 'Role unknown'
              ? 'muted'
              : 'neutral';
        const state = row.querySelector<HTMLElement>('.tmt-ui-list-state')!;
        state.textContent = `${entry.connection.mark} ${entry.connection.text}`;
        state.dataset.tone = entry.connection.tone;
        const open = row.querySelector<HTMLAnchorElement>('[data-action="open"]')!;
        open.setAttribute('aria-disabled', String(!entry.thisMachine && !!blocked()));
        if (!entry.thisMachine && blocked()) open.removeAttribute('href');
        else open.href = entry.thisMachine ? new URL('./', location.href).href : entry.url;
        open.title = !entry.thisMachine ? blocked() : '';
      }
      menu(projection);
      return;
    }
    rowMenus.forEach((item) => item.dispose());
    rowMenus = [];
    list.replaceChildren(
      ...projection.rows.map((entry) => {
        const row = node('li');
        row.className = browserUiClasses.listRow;
        row.dataset.remote = entry.id;
        row.dataset.thisMachine = String(entry.thisMachine);
        row.dataset.url = entry.url;
        row.dataset.kind = entry.reference?.kind ?? 'local';
        row.dataset.leading = '';
        const leading = node('div');
        leading.className = browserUiClasses.listLeading;
        leading.append(createIcon(entry.backend === 'Local' ? 'terminal' : 'box'));
        const title = node('div');
        title.className = browserUiClasses.listTitle;
        const label = node('span');
        label.className = `${browserUiClasses.listLabel} remote-label`;
        const open = node('a', entry.name);
        open.href = entry.thisMachine ? new URL('./', location.href).href : entry.url;
        open.dataset.action = 'open';
        open.setAttribute('role', 'link');
        open.setAttribute('aria-label', `Open ${entry.name}`);
        open.setAttribute('aria-disabled', String(!entry.thisMachine && !!blocked()));
        if (!entry.thisMachine && blocked()) open.removeAttribute('href');
        open.title = !entry.thisMachine ? blocked() : '';
        open.addEventListener('click', (event) => {
          event.preventDefault();
          if (entry.thisMachine) {
            (shell ?? remoteShell())?.navigate(new URL('./', location.href).href);
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
        if (entry.thisMachine && entry.name !== 'This machine') label.append(chip('This machine'));
        const role = chip(entry.roleLabel);
        role.className += ' remote-role';
        role.dataset.role = '';
        role.dataset.roleTone =
          entry.roleLabel === 'Admin'
            ? 'selected'
            : entry.roleLabel === 'Role unknown'
              ? 'muted'
              : 'neutral';
        label.append(role);
        const description = node(
          'span',
          `${entry.backend}${entry.project ? ` · ${entry.project}` : ''} · ${entry.url}`,
        );
        description.className = browserUiClasses.listDescription;
        title.append(label, description);
        const meta = node('div');
        meta.className = browserUiClasses.listMeta;
        const state = node('div', `${entry.connection.mark} ${entry.connection.text}`);
        state.className = browserUiClasses.listState;
        state.dataset.tone = entry.connection.tone;
        meta.append(state);
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
        row.append(leading, title, meta, actions);
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
  function openAdd(): void {
    const view = dialog('Add remote', join);
    const alias = field('Remote name', '');
    const url = field('Remote address or link', '');
    view.body.append(alias.label, url.label);
    view.buttons.append(
      action('Continue', () => {
        void reference(alias.input.value, url.input.value)
          .then((entry) => {
            // A validation that outlived its dialog is dropped: it never confirms or touches
            // a later opening.
            if (currentDialog !== view.box || !view.box.open || !view.box.isConnected) return;
            view.end();
            confirmAdd(entry, join);
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
      if (!value || ['#remotes', '#add-remote'].includes(value)) return;
      history.replaceState(null, '', location.pathname + location.search);
      receive(value);
    },
    { signal: (shell ?? remoteShell())?.signal },
  );
  const observe = (state: string, checkedAddress?: string, role?: ViewerRole | null): void => {
    observed = state;
    if (state !== 'Connected' || role === null) observedRole = undefined;
    else if (role !== undefined) observedRole = role;
    if (checkedAddress) address = checkedAddress;
    paint();
  };
  return Object.assign(observe, {
    blockedBy: (reason: () => string): void => {
      heldBy = reason;
    },
  });
}
