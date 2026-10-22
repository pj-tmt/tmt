import { remoteShell, type RemoteShell } from './remote-shell.js';
import { remoteMenu } from './remote-menu.js';
import { remoteState, currentRemote } from './remote-state.js';
/** Remote pages' public-reference consumer. It never probes a foreign origin or shares a key. */
import {
  remoteRegistry,
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
function action(text: string, run: () => void): HTMLButtonElement {
  const button = node('button', text);
  button.type = 'button';
  button.className = 'tmt-ui-action';
  button.addEventListener('click', run);
  return button;
}
function field(label: string, value: string): { label: HTMLLabelElement; input: HTMLInputElement } {
  const wrapper = node('label', label);
  wrapper.className = 'tmt-ui-field-label';
  const input = node('input');
  input.className = 'tmt-ui-field-control';
  input.value = value;
  wrapper.append(input);
  return { label: wrapper, input };
}

function kindLabel(entry: RemoteReference): string {
  return entry.kind === 'local' ? 'Local' : 'Firebase';
}

/** The page's registry controller: observations in, and the reason a switch is held (settings). */
export type RemoteRegistryPage = ((state: string, address?: string) => void) & {
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
  const add = document.getElementById('remote-add') as HTMLButtonElement;
  let observed = 'Checking';
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
      open: (entry) => {
        if (blocked()) return;
        void remoteRegistry
          .navigation(entry)
          .then((url) => location.assign(url))
          .catch((error: unknown) => showError(error, status));
      },
      add: () => {
        // The registry lives on the settings view: go there (the same route, so the page only
        // shows another view), then open the form.
        if (document.documentElement.dataset.tmtPage !== 'settings')
          (shell ?? remoteShell())?.navigate(settings.href);
        add.click();
      },
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
      blocked,
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
    box.className = 'remote-dialog';
    const heading = node('h2', title);
    heading.id = 'remote-dialog-heading';
    box.setAttribute('aria-labelledby', heading.id);
    const body = node('div');
    const message = node('p');
    message.setAttribute('role', 'status');
    const buttons = node('div');
    buttons.className = 'remote-actions';
    box.append(heading, body, message, buttons);
    root.append(box);
    const end = (): void => box.close();
    box.addEventListener(
      'close',
      () => {
        box.remove();
        if (currentDialog === box) {
          currentDialog = undefined;
          (opener.isConnected ? opener : add).focus();
        }
      },
      { once: true },
    );
    currentDialog = box;
    return { box, body, buttons, message, end };
  };
  const save = async (run: () => Promise<void>, view: ReturnType<typeof dialog>): Promise<void> => {
    const buttons = [...view.box.querySelectorAll('button')];
    buttons.forEach((b) => {
      b.disabled = true;
    });
    try {
      await run();
      await render();
      view.end();
    } catch (error) {
      showError(error, view.message);
    } finally {
      buttons.forEach((b) => {
        b.disabled = false;
      });
    }
  };
  const importView = (plan: ImportPlan): void => {
    const view = dialog('Add these remotes to this browser?', add);
    view.body.append(
      node('p', 'Only names and addresses are copied. No keys, pairings or drafts.'),
    );
    const choices: (() => string | null)[] = [];
    for (const row of plan.rows) {
      const item = node('section');
      item.className = 'remote-import-row';
      item.append(
        node('h3', row.entry.name),
        node('p', `${kindLabel(row.entry)} · ${row.entry.url}`),
      );
      if (row.status === 'duplicate') {
        item.append(node('p', `Already added as ${row.existingName}`));
        choices.push(() => null);
      } else if (row.status === 'collision') {
        item.append(node('p', `You already have a remote named ${row.entry.name}`));
        const alias = field('Remote name', row.suggestedName);
        const skip = node('input');
        skip.type = 'checkbox';
        const skipLabel = node('label', 'Skip');
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
    view.body.append(
      node('p', entry.kind === 'firestore' ? `${entry.project}.web.app` : entry.url),
    );
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
    const view = dialog(`Remove ${entry.name} from this browser?`, opener);
    view.body.append(
      node('p', 'This only removes the entry here. It does not sign out or revoke access.'),
    );
    view.buttons.append(
      action('Remove', () => {
        void save(() => remoteRegistry.remove(entry.name), view);
      }),
      action('Cancel', view.end),
    );
    view.box.showModal();
  };
  const current = (entry: RemoteReference): boolean => currentRemote(entry, address, location.href);
  const render = async (): Promise<void> => {
    try {
      catalog = await remoteRegistry.list();
      const rows = catalog.entries.map((entry) => {
        const row = node('li');
        row.dataset.remote = entry.name;
        row.dataset.url = entry.url;
        row.dataset.kind = entry.kind;
        row.append(node('h3', entry.name), node('p', `${kindLabel(entry)} · ${entry.url}`));
        const state = node('p', remoteState(observed, current(entry)).text);
        state.className = 'remote-state';
        state.dataset.tone = remoteState(observed, current(entry)).tone;
        row.append(state);
        const actions = node('div');
        actions.className = 'remote-actions';
        const open = action('Open', () => {
          void (async () => {
            try {
              if (!blocked()) location.assign(await remoteRegistry.navigation(entry));
            } catch (error) {
              showError(error, status);
            }
          })();
        });
        open.dataset.action = 'open';
        const renameButton = action('Rename', () => rename(entry, renameButton));
        const removeButton = action('Remove', () => remove(entry, removeButton));
        renameButton.setAttribute('aria-label', `Rename ${entry.name}`);
        removeButton.setAttribute('aria-label', `Remove ${entry.name}`);
        if (!current(entry)) {
          open.disabled = !!blocked();
          open.title = blocked();
          actions.append(open);
        }
        actions.append(renameButton, removeButton);
        row.append(actions);
        return row;
      });
      list.replaceChildren(...rows);
      if (!rows.length) list.append(node('li', 'No remotes added in this browser.'));
      menu(catalog, observed, address ?? '');
    } catch (error) {
      showError(error, status);
    }
  };
  add.addEventListener('click', () => {
    const view = dialog('Add remote', add);
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
            confirmAdd(entry, add);
          })
          .catch((error: unknown) => showError(error, view.message));
      }),
      action('Cancel', view.end),
    );
    view.box.showModal();
  });
  void render().then(() => {
    if (fragment === '#add-remote') add.click();
    else if (fragment === '#remotes') {
      root.scrollIntoView();
      add.focus({ preventScroll: true });
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
  const observe = (state: string, checkedAddress?: string): void => {
    observed = state;
    if (checkedAddress) address = checkedAddress;
    for (const row of list.querySelectorAll<HTMLElement>('[data-remote]')) {
      const stateNode = row.querySelector<HTMLElement>('.remote-state')!;
      const entry = catalog.entries.find((entry) => entry.name === row.dataset.remote);
      const isCurrent = !!entry && currentRemote(entry, address, location.href);
      const view = remoteState(observed, isCurrent);
      stateNode.textContent = view.text;
      stateNode.dataset.tone = view.tone;
      const open = row.querySelector<HTMLButtonElement>('[data-action="open"]');
      if (open) {
        open.disabled = !!blocked();
        open.title = blocked();
      }
      if (isCurrent) row.querySelector('[data-action="open"]')?.remove();
    }
    menu(catalog, observed, address ?? '');
  };
  return Object.assign(observe, {
    blockedBy: (reason: () => string): void => {
      heldBy = reason;
    },
  });
}
