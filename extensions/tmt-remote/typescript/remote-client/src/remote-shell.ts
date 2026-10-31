/** Remote's routes inside the one TMT web app (design/browser-ui `shell-host.ts`). */
import type { ShellAnswer, ShellMenu } from '@tmt/browser-ui/shell';
import type { ShellSlot } from '@tmt/browser-ui/shell-host';

type Select = (id: string, href: string) => boolean;
export interface RemoteShell {
  /** The module menu this page shows; `select` takes over a plain click on one of its items. */
  setMenu(menu: ShellMenu | undefined, select?: Select): void;
  /** Go to another TMT URL without reloading; the same route only follows the new path. */
  navigate(href: string): void;
  /** Route identity in the top bar and the shell-owned document title. */
  setTitle(
    title: string,
    ancestors?: readonly string[],
    documentTitle?: string,
    chips?: readonly string[],
  ): void;
  /**
   * Read the owner's composed shell once: at `address` when the page already holds the door's
   * answer, otherwise through the door. Pages call it only when it may send work; the landing
   * calls it after its own check connects, so an unpaired landing sends nothing.
   */
  describe(address?: string): void;
  /** Called once the composed shell is read: the answer, or undefined without an owner session. */
  described(listener: (answer: ShellAnswer | undefined) => void): void;
  /** A verified Session replacement invalidates root-owned search state, never grants authority. */
  observedSession?(session: object): void;
  /** Reuse the settings model's existing hold reason for root search navigation. */
  holdNavigation?(reason: () => string): void;
  /** Aborted when the route unmounts: page code ends its listeners, checks and sessions on it. */
  readonly signal: AbortSignal;
}

/** Routes supply plain title segments; the shared leaf owns layout and phone truncation. */
export function setRouteTitle(
  slot: Pick<ShellSlot, 'setTitle' | 'topBarReady' | 'setTopBar'>,
  title: string,
  ancestors: readonly string[] = [],
  documentTitle = title,
  chips: readonly string[] = [],
): void {
  if (!slot.topBarReady()) return;
  slot.setTitle(documentTitle);
  const target = slot.setTopBar().title;
  const label = [...ancestors, title].join(' / ');
  if (
    target.querySelector('h1')?.getAttribute('aria-label') === label &&
    (target.querySelector('.tmt-ui-topbar-title-chips')?.textContent ?? '') === chips.join('')
  )
    return;
  if (target.querySelector('h1')?.getAttribute('aria-label') !== label) {
    const heading = document.createElement('h1');
    heading.setAttribute('aria-label', label);
    heading.tabIndex = -1;
    if (ancestors.length) {
      const prefix = document.createElement('span');
      prefix.className = 'tmt-ui-topbar-title-prefix';
      prefix.textContent = `${ancestors.join(' / ')} / `;
      heading.append(prefix);
    }
    const current = document.createElement('span');
    current.className = 'tmt-ui-topbar-title-current';
    current.textContent = title;
    heading.append(current);
    target.replaceChildren(heading);
  }
  const group = document.createElement('span');
  group.className = 'tmt-ui-topbar-title-chips';
  for (const text of chips) {
    const chip = document.createElement('span');
    chip.className = 'tmt-ui-chip';
    const label = document.createElement('span');
    label.className = 'tmt-ui-chip-label';
    label.textContent = text;
    chip.append(label);
    group.append(chip);
  }
  target.querySelector('.tmt-ui-topbar-title-chips')?.remove();
  if (chips.length) target.append(group);
}

let current: RemoteShell | undefined;

/** The mounted route's part of the one shell, or undefined outside the app (unit tests). */
export function remoteShell(): RemoteShell | undefined {
  return current;
}

/** Give Remote's page code the mounted route's slot and lifetime; `undefined` when it unmounts. */
export function hostRemote(
  slot: ShellSlot | undefined,
  signal?: AbortSignal,
  observedSession?: (session: object) => void,
  holdNavigation?: (reason: () => string) => void,
): void {
  current = slot &&
    signal && {
      setMenu: (menu, select) => slot.setMenu(menu, select),
      navigate: (href) => slot.navigate(href),
      setTitle: (title, ancestors, documentTitle, chips) =>
        setRouteTitle(slot, title, ancestors, documentTitle, chips),
      describe: (address) => slot.describe(address),
      described: (listener) => slot.described(listener),
      observedSession: (session) => {
        if (!signal.aborted) observedSession?.(session);
      },
      holdNavigation: (reason) => {
        if (!signal.aborted) holdNavigation?.(reason);
      },
      signal,
    };
}

/** A shared list row (`tmt-ui-list*`): mark, linked title, then its detail as metadata. */
function row(href: string, mark: string, title: string, detail: string): HTMLLIElement {
  const item = document.createElement('li');
  item.className = 'tmt-ui-list-row';
  item.dataset.leading = '';
  const sign = document.createElement('div');
  sign.className = 'tmt-ui-list-leading';
  sign.setAttribute('aria-hidden', 'true');
  sign.textContent = mark;
  if (mark === '◆') sign.dataset.tone = 'waiting';
  const name = document.createElement('div');
  name.className = 'tmt-ui-list-title';
  const link = document.createElement('a');
  link.href = href;
  link.textContent = title;
  name.append(link);
  const meta = document.createElement('div');
  meta.className = 'tmt-ui-list-meta';
  const more = document.createElement('div');
  more.className = 'tmt-ui-list-updated';
  more.textContent = detail;
  meta.append(more);
  item.append(sign, name, meta);
  return item;
}

/** One Needs-you row per module with a count and a waiting pairing offer; no per-item list until
 * modules publish items. Recent pages are the modules' described home cards. `open` turns a
 * module's described path into where this app opens it. */
export function homePage(shell: RemoteShell, open: (href: string) => string): void {
  const needs = document.getElementById('home-needs');
  const note = document.getElementById('home-needs-note');
  const recent = document.getElementById('home-recent');
  const recentBlock = document.getElementById('home-recent-block');
  if (!needs || !note || !recent || !recentBlock) return;
  const offer = fetch('/sdk/pair-offer', { cache: 'no-store' })
    .then((response) => response.status === 200)
    .catch(() => false);
  shell.described(async (answer) => {
    const rows: HTMLLIElement[] = [];
    for (const module of answer?.modules ?? [])
      if (module.needsYou)
        rows.push(
          row(
            open(module.rail.path),
            '◆',
            `${module.needsYou} in ${module.rail.label}`,
            'Needs you',
          ),
        );
    if (await offer)
      rows.push(
        row(
          new URL('/remote/pair', location.href).href,
          '◆',
          'A device is waiting to pair',
          'Remote · confirm the four words in the terminal',
        ),
      );
    needs.replaceChildren(...rows);
    if (answer) note.textContent = rows.length ? '' : 'Nothing needs you.';
    else {
      const pair = Object.assign(document.createElement('a'), {
        href: '/remote/',
        textContent: 'Pair this browser',
      });
      note.replaceChildren(pair, ' in Remote to see what needs you.');
    }
    note.hidden = !note.textContent;
    const cards = (answer?.modules ?? []).flatMap((module) =>
      (module.home ?? []).map((card) => row(open(card.path), '▤', card.title, module.rail.label)),
    );
    recent.replaceChildren(...cards);
    recentBlock.hidden = cards.length === 0;
  });
}
