import type { Session } from './device.js';
import { members, type MemberSummary } from './members.js';
/** One mounted Remote model and persistent tab panels; paths are declared in remote-routes. */
import { remoteRegistryPage, type RemoteRegistryPage } from './remote-registry-page.js';
import { REMOTE_ITEMS, remoteItem, remoteTabAccess, type RemoteItem } from './remote-routes.js';
import type { RemoteRegistryView } from './remote-registry.js';
import type { RemoteShell } from './remote-shell.js';
import { commandCopy } from './command-copy.js';
import { joinPage } from './join-page.js';
import { invitationFromInput } from './invite-page.js';
import { createIcon } from '@tmt/browser-ui/glyph';
import { browserUiClasses as c } from '@tmt/browser-ui/static';
import { createPageSubheader } from '@tmt/browser-ui/list-table';

export interface RemotePages {
  landing(
    lifetime: AbortSignal,
    registry: RemoteRegistryPage,
    connected: (session: Session) => void,
  ): Promise<void>;
  settings(
    shell: RemoteShell,
    registry: RemoteRegistryPage,
    initial?: Session,
  ): Promise<(session: Session) => void>;
}
export async function remoteApp(
  shell: RemoteShell,
  main: HTMLElement,
  bar: HTMLElement,
  settingsStyles: () => Promise<void>,
  pages: () => Promise<RemotePages>,
): Promise<void> {
  // The private invitation fragment is erased before registry handoff can inspect it.
  function takeJoinFragment(): string {
    if (location.pathname !== '/remote/join' || !location.hash) return '';
    const secret = location.hash.slice(1);
    history.replaceState(null, '', location.pathname + location.search);
    if (location.hash) throw new Error('Could not clear invitation fragment.');
    return secret;
  }
  let initialInvitation = '';
  let initialInvalid = false;
  let initialTimer: ReturnType<typeof setTimeout> | undefined;
  let unsafeFragment = false;
  const clearInitial = () => {
    initialInvitation = '';
    initialInvalid = false;
    if (initialTimer !== undefined) clearTimeout(initialTimer);
    initialTimer = undefined;
  };
  const capturePending = () => {
    if (location.pathname !== '/remote/join') {
      clearInitial();
      return;
    }
    if (!location.hash) return;
    clearInitial();
    try {
      const selected = invitationFromInput(takeJoinFragment());
      initialInvitation = selected.token;
      initialTimer = setTimeout(
        () => {
          initialInvitation = '';
          initialInvalid = true;
        },
        Math.max(0, selected.expiry - Date.now()),
      );
    } catch {
      clearInitial();
      initialInvalid = true;
      // A failed erase must stop before the registry sees the private fragment.
      unsafeFragment = !!location.hash;
    }
  };
  capturePending();
  window.addEventListener('hashchange', capturePending, { signal: shell.signal });
  window.addEventListener('popstate', capturePending, { signal: shell.signal });
  shell.signal.addEventListener('abort', clearInitial, { once: true });
  await settingsStyles();
  if (shell.signal.aborted) return;
  capturePending();
  window.removeEventListener('hashchange', capturePending);
  window.removeEventListener('popstate', capturePending);
  if (unsafeFragment) return;
  const models = [...main.querySelectorAll<HTMLElement>(':scope > [data-page]')];
  const registry = remoteRegistryPage(undefined, shell);
  const listActions = main.querySelector<HTMLElement>('.remote-head-actions');
  // The list already owns its content action row; its identity is now in the common bar.
  main.querySelector('#remote-head .settings-title')?.remove();
  const started = new Map<RemoteItem['page'], Promise<void>>();
  let shown: RemoteItem | undefined;
  let observed: RemoteRegistryView | undefined;
  let memberSummary: MemberSummary | undefined;
  let admitted: Session | undefined;
  let summarySession: Session | undefined;
  let summaryGeneration = 0;
  let observeSession: ((session: Session) => void) | undefined;
  const tabs = REMOTE_ITEMS.filter((item) => !['remotes', 'create', 'join'].includes(item.id));
  const frame = document.createElement('div');
  frame.id = 'remote-context';
  frame.className = `tmt-page remote-context ${c.tabs}`;
  frame.dataset.variant = 'flat';
  const needsYou = document.createElement('p');
  needsYou.id = 'remote-needs-you';
  needsYou.hidden = true;
  let reportedNeeds: number | undefined;
  const designation = document.createElement('p');
  designation.id = 'remote-designation';
  const strip = document.createElement('div');
  strip.className = c.tabList;
  strip.setAttribute('role', 'tablist');
  strip.setAttribute('aria-label', 'This Remote');
  const buttons = new Map<string, HTMLButtonElement>();
  const panels = new Map<string, HTMLElement>();
  const globalPanel = document.createElement('div');
  const subheaders = new Map<RemoteItem['id'], HTMLElement>();
  // People and Invites share the one Invite action; layout moves it into the shown subheader.
  const invite = main.querySelector<HTMLElement>('#remote-invite');
  const inviteSlots = new Map<RemoteItem['id'], HTMLElement>();
  for (const item of REMOTE_ITEMS) {
    if (
      !['create', 'join', 'members', 'invites', 'my-devices', 'service', 'settings'].includes(
        item.id,
      )
    )
      continue;
    const slot = ['members', 'invites'].includes(item.id)
      ? document.createElement('span')
      : undefined;
    if (slot) inviteSlots.set(item.id, slot);
    subheaders.set(
      item.id,
      createPageSubheader({
        title: item.title,
        ...(item.id === 'members' ? { subtitle: 'Sending is off until an admin enables it.' } : {}),
        ...(item.id === 'invites'
          ? { subtitle: 'Invitations from the last 7 days · a code works once and expires' }
          : {}),
        ...(slot ? { actions: slot } : {}),
      }),
    );
  }
  document.getElementById('firestore-title')?.classList.add('tmt-ui-visually-hidden');
  for (const item of tabs) {
    const button = document.createElement('button');
    button.id = `remote-tab-${item.id}`;
    button.className = c.tab;
    button.type = 'button';
    button.setAttribute('role', 'tab');
    button.setAttribute('aria-controls', `remote-panel-${item.id}`);
    button.addEventListener('click', () => shell.navigate(new URL(item.path, location.href).href), {
      signal: shell.signal,
    });
    button.addEventListener(
      'keydown',
      (event) => {
        const index = tabs.indexOf(item);
        const next =
          event.key === 'Home'
            ? tabs[0]
            : event.key === 'End'
              ? tabs.at(-1)
              : event.key === 'ArrowRight'
                ? tabs[(index + 1) % tabs.length]
                : event.key === 'ArrowLeft'
                  ? tabs[(index + tabs.length - 1) % tabs.length]
                  : undefined;
        if (!next) return;
        event.preventDefault();
        shell.navigate(new URL(next.path, location.href).href);
        // Navigation can be refused by a retained draft; never steal focus from that explanation.
        if (remoteItem(location.pathname)?.id === next.id) buttons.get(next.id)?.focus();
      },
      { signal: shell.signal },
    );
    buttons.set(item.id, button);
    strip.append(button);
    const panel = document.createElement('div');
    panel.id = `remote-panel-${item.id}`;
    panel.className = `${c.tabPanel} remote-tab-content`;
    panel.setAttribute('role', 'tabpanel');
    panel.setAttribute('aria-labelledby', button.id);
    panel.hidden = true;
    panels.set(item.id, panel);
  }
  const locked = document.createElement('section');
  locked.id = 'remote-locked';
  locked.className = 'tmt-page';
  const lockTitle = document.createElement('h2');
  lockTitle.textContent = 'Locked';
  const lockReason = document.createElement('p');
  const request = document.createElement('div');
  locked.append(lockTitle, lockReason, request);
  let requestText = '';
  frame.append(designation, needsYou, strip);
  main.prepend(frame, globalPanel, ...panels.values());

  function keepTabVisible(): void {
    const button = shown && buttons.get(shown.id);
    if (!button || frame.hidden) return;
    const delta = button.getBoundingClientRect().left - strip.getBoundingClientRect().left;
    if (delta < 0) strip.scrollLeft += delta;
    else if (delta + button.offsetWidth > strip.clientWidth)
      strip.scrollLeft += delta + button.offsetWidth - strip.clientWidth;
  }
  const resize = new ResizeObserver(keepTabVisible);
  resize.observe(strip);
  shell.signal.addEventListener('abort', () => resize.disconnect(), { once: true });
  function layout(): void {
    if (!shown) return;
    main.dataset.remoteView = shown.id;
    if (listActions) listActions.hidden = shown.id !== 'remotes';
    const localName = document.getElementById('remote-local-name-value');
    if (localName) localName.textContent = observed?.current.name ?? 'This machine';
    const renameLabel = document.querySelector('#remote-local-rename .tmt-ui-action-label');
    if (renameLabel)
      renameLabel.textContent = observed?.current.reference
        ? 'Rename in this browser'
        : 'Remember this Remote';
    const role = observed?.current.role;
    const access = remoteTabAccess(shown, role);
    needsYou.hidden = shown.id !== 'overview' || reportedNeeds === undefined;
    needsYou.textContent =
      reportedNeeds === undefined
        ? ''
        : reportedNeeds === 0
          ? 'Nothing needs you.'
          : `${reportedNeeds} ${reportedNeeds === 1 ? 'item needs' : 'items need'} your attention.`;
    const global = ['remotes', 'create', 'join'].includes(shown.id);
    frame.hidden = global;
    shell.setTitle(
      shown.title,
      global
        ? ['Remote']
        : [
            'Remote',
            `${observed?.current.name ?? 'This machine'} · ${observed?.current.roleLabel ?? 'Role unknown'}`,
          ],
    );
    designation.textContent =
      role === 'owner'
        ? 'This browser manages this Remote.'
        : role === 'user'
          ? 'Read-only in this browser. Ask the admin to make changes.'
          : 'Access is not confirmed. Check again before making changes.';
    for (const item of tabs) {
      const button = buttons.get(item.id)!;
      const policy = remoteTabAccess(item, role);
      const selected = item === shown;
      button.setAttribute('aria-selected', String(selected));
      button.tabIndex = selected ? 0 : -1;
      const label = policy.locked ? `Locked · ${item.label}` : item.label;
      if (button.dataset.tabLabel !== label) {
        button.dataset.tabLabel = label;
        button.replaceChildren();
        if (policy.locked) button.append(createIcon('lock'));
        const text = document.createElement('span');
        text.className = c.tabLabel;
        text.textContent = label;
        button.append(text);
      }
      if (item.id === 'members') {
        const previous = button.querySelector(`.${c.tabCount}`);
        const total = memberSummary?.memberCount;
        if (total == null) {
          previous?.remove();
          button.removeAttribute('aria-label');
        } else {
          const count = previous ?? document.createElement('span');
          count.className = c.tabCount;
          count.textContent = String(total);
          count.setAttribute('aria-hidden', 'true');
          if (!previous) button.append(count);
          button.setAttribute(
            'aria-label',
            `${label} · ${total} ${total === 1 ? 'person' : 'people'}`,
          );
        }
      }
      panels.get(item.id)!.hidden = !selected;
    }
    keepTabVisible();
    globalPanel.hidden = !global;
    const panel = global ? globalPanel : panels.get(shown.id)!;
    const header = subheaders.get(shown.id);
    const peopleSubtitle = subheaders.get('members')?.querySelector('p');
    if (peopleSubtitle) {
      const total = memberSummary?.memberCount;
      peopleSubtitle.textContent = `${total == null ? '' : `${total} ${total === 1 ? 'person' : 'people'} · `}Sending is off until an admin enables it.`;
    }
    for (const [id, value] of subheaders) value.hidden = id !== shown.id;
    const inviteSlot = inviteSlots.get(shown.id);
    if (invite && inviteSlot && invite.parentElement !== inviteSlot) inviteSlot.append(invite);
    if (header && header.parentElement !== panel) panel.prepend(header);
    // Move the same retained models, never clone/restart their forms or original outcomes.
    for (const model of models) {
      if (model.parentElement !== panel) panel.append(model);
      model.hidden = model.dataset.page !== shown.page;
    }
    for (const part of main.querySelectorAll<HTMLElement>('[data-view]'))
      part.hidden = !part.dataset.view!.split(' ').includes(shown.id);
    if (access.locked) {
      if (locked.parentElement !== panel) panel.prepend(locked);
      locked.hidden = false;
      lockReason.textContent = access.reason;
      if (requestText !== access.request) {
        requestText = access.request;
        request.replaceChildren(...(requestText ? [commandCopy(requestText)] : []));
      }
    } else locked.hidden = true;
  }
  const localRename = document.getElementById('remote-local-rename');
  localRename?.addEventListener('click', () => registry.renameCurrent(localRename), {
    signal: shell.signal,
  });
  shell.described((answer) => {
    if (shell.signal.aborted) return;
    reportedNeeds = answer?.modules.find(
      (module) => module.extension === 'remote' && !('module' in module),
    )?.needsYou;
    layout();
  });
  function observeSummary(): void {
    const session = admitted;
    if (
      !session ||
      observed?.current.role !== 'owner' ||
      observed.current.connection.text !== 'Connected'
    ) {
      if (summarySession) {
        summarySession = undefined;
        summaryGeneration++;
      }
      return;
    }
    if (summarySession === session) return;
    summarySession = session;
    const generation = ++summaryGeneration;
    void members(session)
      .summary()
      .then((summary) => {
        if (admitted === session && summaryGeneration === generation && !shell.signal.aborted)
          registry.setSummary(summary);
      })
      .catch(() => {
        if (admitted === session && summaryGeneration === generation)
          registry.setSummary(undefined);
      });
  }
  const unsubscribe = registry.watch((view) => {
    observed = view;
    observeSummary();
    layout();
  });
  shell.signal.addEventListener('abort', unsubscribe, { once: true });
  const stopSummary = registry.watchSummary((value) => {
    memberSummary = value;
    layout();
  });
  shell.signal.addEventListener('abort', stopSummary, { once: true });
  function start(page: RemoteItem['page']): Promise<void> {
    let run = started.get(page);
    if (!run) {
      run = (async () => {
        const code = await pages();
        if (shell.signal.aborted) return;
        if (page === 'landing')
          await code.landing(shell.signal, registry, (session) => {
            admitted = session;
            summarySession = undefined;
            summaryGeneration++;
            registry.setSummary(undefined);
            observeSummary();
            shell.observedSession?.(session);
            if (observeSession) observeSession(session);
            else void start('settings');
          });
        else observeSession = await code.settings(shell, registry, admitted);
      })();
      started.set(page, run);
    }
    return run;
  }
  let disposeJoin: (() => void) | undefined;
  shell.signal.addEventListener('abort', () => disposeJoin?.(), { once: true });
  async function show(): Promise<void> {
    const item = remoteItem(location.pathname);
    if (!item) return;
    const changed = shown !== item;
    const first = !shown;
    if (changed) {
      disposeJoin?.();
      disposeJoin = undefined;
      if (item.id === 'join') {
        const target = main.querySelector<HTMLElement>('#remote-join-guide');
        if (target) {
          let secret = initialInvitation || takeJoinFragment();
          let invalid = initialInvalid;
          clearInitial();
          if (secret) {
            try {
              invitationFromInput(secret);
            } catch {
              secret = '';
              invalid = true;
            }
          }
          disposeJoin = joinPage(
            target,
            secret,
            () => shell.navigate(new URL('/remote/', location.href).href),
            invalid,
          );
        }
      } else clearInitial();
    }
    shown = item;
    document.documentElement.dataset.tmtPage = item.page;
    layout();
    if (changed && !first) window.scrollTo({ top: 0 });
    // The same bounded management observation supplies role on every current-door tab.
    await start('landing');
    if (!shell.signal.aborted) await start('settings');
  }
  window.addEventListener('popstate', () => void show().catch(() => {}), { signal: shell.signal });
  window.addEventListener(
    'hashchange',
    () => {
      if (location.pathname !== '/remote/join' || !location.hash) return;
      const secret = takeJoinFragment();
      disposeJoin?.();
      const target = main.querySelector<HTMLElement>('#remote-join-guide');
      if (target)
        disposeJoin = joinPage(target, secret, () =>
          shell.navigate(new URL('/remote/', location.href).href),
        );
    },
    { signal: shell.signal },
  );
  shell.signal.addEventListener(
    'abort',
    () => {
      delete main.dataset.remoteView;
    },
    { once: true },
  );
  await show();
}
