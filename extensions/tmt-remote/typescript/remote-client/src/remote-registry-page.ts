/** Landing's public-reference consumer. It never probes a foreign origin or shares a key. */
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

export function landingRegistry(): (state: string, address?: string) => void {
  const root = document.getElementById('remotes');
  if (!root) return () => {};
  // Remove the entire untrusted fragment synchronously, before any admission can begin.
  const fragment = location.hash;
  if (fragment) history.replaceState(null, '', location.pathname + location.search);
  const list = document.getElementById('remote-list')!;
  const status = document.getElementById('remote-status')!;
  const add = document.getElementById('remote-add') as HTMLButtonElement;
  let observed = 'Checking';
  let address: string | undefined;
  let currentDialog: HTMLDialogElement | undefined;

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
  const current = (entry: RemoteReference): boolean =>
    entry.kind === 'local' &&
    (entry.url === address || new URL(entry.url).href === `${location.origin}/`);
  const render = async (): Promise<void> => {
    try {
      const catalog = await remoteRegistry.list();
      const rows = catalog.entries.map((entry) => {
        const row = node('li');
        row.dataset.remote = entry.name;
        row.dataset.url = entry.url;
        row.dataset.kind = entry.kind;
        row.append(node('h3', entry.name), node('p', `${kindLabel(entry)} · ${entry.url}`));
        const state = node(
          'p',
          current(entry) ? (observed === 'Connected' ? '● Connected' : observed) : 'not checked',
        );
        state.className = 'remote-state';
        row.append(state);
        const actions = node('div');
        actions.className = 'remote-actions';
        const open = action('Open', () => {
          void (async () => {
            try {
              location.assign(await remoteRegistry.navigation(entry));
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
        if (!current(entry)) actions.append(open);
        actions.append(renameButton, removeButton);
        row.append(actions);
        return row;
      });
      list.replaceChildren(...rows);
      if (!rows.length) list.append(node('li', 'No remotes added in this browser.'));
    } catch (error) {
      showError(error, status);
    }
  };
  add.addEventListener('click', () => {
    const view = dialog('Add remote', add);
    const alias = field('Remote name', '');
    const url = field('Door address or remote link', '');
    view.body.append(alias.label, url.label);
    view.buttons.append(
      action('Continue', () => {
        void reference(alias.input.value, url.input.value)
          .then((entry) => {
            view.end();
            confirmAdd(entry, add);
          })
          .catch((error: unknown) => showError(error, view.message));
      }),
      action('Cancel', view.end),
    );
    view.box.showModal();
  });
  void render();
  const receive = (value: string): void => {
    void remoteRegistry
      .parseHandoff(value)
      .then(async (packet) => {
        const plan = await remoteRegistry.planImport(packet.catalog);
        importView(plan);
      })
      .catch((error: unknown) => showError(error, status));
  };
  if (fragment) receive(fragment);
  window.addEventListener('hashchange', () => {
    const value = location.hash;
    if (!value) return;
    history.replaceState(null, '', location.pathname + location.search);
    receive(value);
  });
  return (state, checkedAddress) => {
    observed = state;
    if (checkedAddress) address = checkedAddress;
    for (const row of list.querySelectorAll<HTMLElement>('[data-remote]')) {
      const stateNode = row.querySelector<HTMLElement>('.remote-state')!;
      const target = row.dataset.url;
      const isCurrent =
        row.dataset.kind === 'local' && (target === address || target === `${location.origin}/`);
      stateNode.textContent = isCurrent
        ? observed === 'Connected'
          ? '● Connected'
          : observed
        : 'not checked';
      if (isCurrent) row.querySelector('[data-action="open"]')?.remove();
    }
  };
}
