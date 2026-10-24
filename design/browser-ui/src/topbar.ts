import { createIcon } from './glyph.js';
import { bindIconActionTooltip } from './icon-action-tooltip.js';
import { mountBrowserMenu } from './menu-host.js';
import { browserUiClasses as c, countText } from './static.js';
import { setStoredTheme, THEMES, type ThemeChoice } from './theme.js';

export type ShellRemoteStatus =
  | { kind: 'confirmed'; liveSessions: number; href: string }
  | { kind: 'unconfirmed'; href: string };

/** Root-owned bindings and display data; the leaf performs no product reads. */
export interface ShellCommonControls {
  openSearch(opener: HTMLButtonElement): void;
  remote: ShellRemoteStatus;
}

/** A route receives only its action slot, never the common-control owner. */
export interface ShellTopBar {
  readonly actions: HTMLElement;
}

/** Internal renderer shared by native and React shells. */
export function createShellTopBar(root: HTMLElement, bar: Element, navigation: HTMLButtonElement) {
  const document = root.ownerDocument;
  const view = document.defaultView!;
  let options: ShellCommonControls | undefined;
  let disposed = false;
  let active: { slot: ShellTopBar; update(): void; dispose(): void } | undefined;
  const validate = (next: ShellCommonControls) => {
    const remote = next.remote;
    if (
      remote.kind === 'confirmed' &&
      (!Number.isSafeInteger(remote.liveSessions) || remote.liveSessions < 0)
    )
      throw new Error('Remote live-session count must be a non-negative integer');
  };
  const clear = () => {
    active?.dispose();
    active = undefined;
  };
  function mount() {
    const header = document.createElement('header');
    header.className = 'tmt-ui-topbar';
    header.setAttribute('aria-label', 'TMT');
    const left = document.createElement('div');
    left.className = 'tmt-ui-topbar-navigation';
    left.append(navigation);
    const right = document.createElement('div');
    right.className = 'tmt-ui-topbar-controls';
    const actions = document.createElement('div');
    actions.className = 'tmt-ui-topbar-actions';
    const common = document.createElement('div');
    common.className = 'tmt-ui-topbar-common';
    common.setAttribute('role', 'group');
    common.setAttribute('aria-label', 'TMT controls');
    const theme = document.createElement('span');
    const search = document.createElement('button');
    search.type = 'button';
    search.className = `${c.action} ${c.iconActionControl} tmt-ui-topbar-search`;
    search.dataset.variant = 'text';
    search.setAttribute('aria-label', 'Search TMT');
    search.setAttribute('aria-haspopup', 'dialog');
    const command = /Mac|iPhone|iPad|iPod/.test(view.navigator.platform);
    search.setAttribute('aria-keyshortcuts', command ? 'Meta+K' : 'Control+K');
    const searchMark = document.createElement('span');
    searchMark.className = c.iconActionIcon;
    searchMark.setAttribute('aria-hidden', 'true');
    const hint = document.createElement('kbd');
    hint.textContent = command ? '⌘K' : 'Ctrl K';
    searchMark.append(createIcon('search', document), hint);
    search.append(searchMark);
    const remote = document.createElement('a');
    remote.className = `${c.action} ${c.iconActionControl} tmt-ui-topbar-remote`;
    remote.dataset.variant = 'text';
    const remoteMark = document.createElement('span');
    remoteMark.className = c.iconActionIcon;
    remoteMark.setAttribute('aria-hidden', 'true');
    remoteMark.append(createIcon('monitor-smartphone', document));
    const badge = document.createElement('span');
    badge.className = c.count;
    badge.setAttribute('aria-hidden', 'true');
    remote.append(remoteMark, badge);

    const help = (control: HTMLElement, label: string) => {
      const wrapper = document.createElement('span');
      wrapper.className = c.iconAction;
      const tooltip = document.createElement('span');
      tooltip.className = c.iconActionTooltip;
      tooltip.setAttribute('popover', 'manual');
      tooltip.setAttribute('aria-hidden', 'true');
      const text = document.createElement('span');
      text.className = c.iconActionTooltipLabel;
      text.textContent = label;
      tooltip.append(text);
      wrapper.append(control, tooltip);
      const binding = bindIconActionTooltip(wrapper, control, tooltip);
      return { wrapper, text, binding };
    };
    const searchHelp = help(search, 'Search TMT');
    const remoteHelp = help(remote, 'Remote: session count unconfirmed');
    common.append(theme, searchHelp.wrapper, remoteHelp.wrapper);
    right.append(actions, common);
    header.append(left, right);

    const choice = (): ThemeChoice => {
      const explicit = document.documentElement.getAttribute('data-theme');
      return explicit === 'light' || explicit === 'dark' ? explicit : 'system';
    };
    const themeOptions = () => ({
      label: `Theme: ${choice()}`,
      icon: createIcon('sun', document),
      items: THEMES.map((key) => ({
        key,
        label: key === 'system' ? 'System' : key === 'light' ? 'Light' : 'Dark',
        checked: choice() === key,
      })),
      onSelect: (key: ThemeChoice) => setStoredTheme(key, document.documentElement),
    });
    const themeMenu = mountBrowserMenu(theme, themeOptions());
    const observeTheme = new MutationObserver(() => themeMenu.update(themeOptions()));
    observeTheme.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-theme'],
    });
    const openSearch = () => {
      themeMenu.close();
      searchHelp.binding.dismiss();
      options!.openSearch(search);
    };
    search.onclick = (event) => {
      if (event.isTrusted) openSearch();
    };
    const shortcut = (event: KeyboardEvent) => {
      if (
        !event.isTrusted ||
        event.defaultPrevented ||
        event.repeat ||
        event.isComposing ||
        event.keyCode === 229 ||
        event.key.toLowerCase() !== 'k' ||
        event.altKey ||
        event.shiftKey ||
        (command ? !event.metaKey || event.ctrlKey : !event.ctrlKey || event.metaKey)
      )
        return;
      const editing = event
        .composedPath()
        .some(
          (node) =>
            node instanceof view.HTMLElement &&
            (node.isContentEditable ||
              node.closest('input,textarea,select,[role="textbox"],[role="combobox"]')),
        );
      if (editing) return;
      event.preventDefault();
      openSearch();
    };
    document.addEventListener('keydown', shortcut);
    root.dataset.topbar = '';
    bar.replaceChildren(header);
    const update = () => {
      const status = options!.remote;
      const label =
        status.kind === 'unconfirmed'
          ? 'Remote: session count unconfirmed'
          : status.liveSessions === 0
            ? 'Remote: no live sessions'
            : `Remote: ${status.liveSessions} live ${status.liveSessions === 1 ? 'session' : 'sessions'}`;
      remote.setAttribute('href', status.href);
      remote.setAttribute('aria-label', label);
      remoteHelp.text.textContent = label;
      badge.hidden = status.kind !== 'confirmed' || status.liveSessions === 0;
      badge.textContent = status.kind === 'confirmed' ? countText(status.liveSessions) : '';
    };
    update();
    return {
      slot: { actions },
      update,
      dispose() {
        document.removeEventListener('keydown', shortcut);
        observeTheme.disconnect();
        themeMenu.dispose();
        searchHelp.binding.dispose();
        remoteHelp.binding.dispose();
        root.prepend(navigation);
        delete root.dataset.topbar;
        header.remove();
      },
    };
  }
  return {
    setCommonControls(next: ShellCommonControls | undefined) {
      if (disposed) return;
      if (next) validate(next);
      options = next;
      if (!next) clear();
      else active?.update();
    },
    setTopBar(): ShellTopBar {
      if (disposed) throw new Error('The shell has been disposed');
      if (!options)
        throw new Error('The root must install common controls before adopting the top bar');
      active ??= mount();
      return active.slot;
    },
    clear,
    dispose() {
      if (disposed) return;
      disposed = true;
      clear();
      options = undefined;
    },
  };
}
