import { brandMarkSvg, freshMarkId } from './brand-mark-svg.js';
import { claimIconActionTooltip } from './icon-action-tooltip.js';
import { moduleIcon, shellIconNodes, type ShellIcon } from './shell-icons.js';
import {
  createShellMenuInteractions,
  type ShellMenuInteractions,
} from './shell-menu-interactions.js';
import { countText } from './static.js';
import { createShellTopBar, type ShellCommonControls, type ShellTopBar } from './topbar.js';
export type { ShellCommonControls, ShellRemoteStatus, ShellTopBar } from './topbar.js';

/**
 * The one TMT web shell (#3937, Colab page c5ccd17a): a full-width global rail,
 * an optional module menu, then the host's own header and content. One framework-free renderer
 * serves the static Remote pages and, through `BrowserShell`, React hosts; labels are always
 * text nodes. The rail is one fixed list, filled from Remote's composed module descriptions.
 */

/** `GET <address>/x/-/shell`, version 1 (contracts/remote-channel-v1.md). Paths are absolute. */
export interface ShellAnswer {
  version: 1;
  modules: ShellModule[];
}
export interface ShellModule {
  extension: string;
  rail: { label: string; icon: string; path: string; order: number };
  menu: { id: string; label: string; path: string; group?: string; count?: ShellAnswerCount }[];
  settings: { id: string; label: string; path: string }[];
  needsYou?: number;
  home?: { id: string; title: string; path: string }[];
}
interface ShellAnswerCount {
  value: number;
  kind: 'needs-you' | 'plain';
}

export interface ShellCount {
  text: string;
  needsYou: boolean;
}
export interface ShellItem {
  id: string;
  label: string;
  icon: ShellIcon;
  href: string;
  count?: ShellCount;
  current: boolean;
  /** An unavailable module stays in the rail, dimmed, with `note` saying why. */
  disabled?: boolean;
  note?: string;
}
export interface ShellMenuItem {
  id: string;
  label: string;
  href: string;
  count?: ShellCount;
  current: boolean;
  /** Plain total beside an attention dot; zero is hidden. */
  total?: number;
  /** Decorative host-selected icon; omitted on ordinary text rows. */
  icon?: ShellIcon;
  /** Up to two host-owned commands, shown on hover/focus without activating this link. */
  actions?: readonly ShellMenuIconAction[];
  /** Shown but not followed; `note` says why (its tooltip and description). */
  disabled?: boolean;
  note?: string;
  /** Bold label, never a colour; the count slot is unchanged. */
  emphasis?: 'unread';
  /** One nested level (a child's own children are not drawn), shown while `expanded`. */
  children?: readonly ShellMenuItem[];
  /** Host-owned disclosure state; the rail never flips it itself. */
  expanded?: boolean;
  /** Requests the opposite `expanded`; the host answers with a new view. */
  toggle?(): void;
}
/** A host-owned command, never a navigation destination or current row. */
export interface ShellMenuAction {
  label: string;
  onActivate(): void;
}
export interface ShellMenuIconAction extends ShellMenuAction {
  icon: ShellIcon;
}
export interface ShellMenuGroup {
  label?: string;
  /** At most one command beside the heading; omitting the icon renders its label as text. */
  action?: ShellMenuAction & { icon?: ShellIcon };
  items: ShellMenuItem[];
  /** Visible and accessible label; the rail adds the decorative plus icon. */
  add?: ShellMenuAction;
  /** Request moving fromId to the current index of toId; the host supplies the resulting order. */
  reorder?(fromId: string, toId: string): void;
}
export interface ShellMenu {
  title: string;
  groups: ShellMenuGroup[];
  /** Module settings, always last after a hairline. */
  settings?: ShellMenuItem;
}
export interface ShellView {
  rail: { main: ShellItem[]; bottom: ShellItem[] };
  menu?: ShellMenu;
}
export interface ShellPlace {
  origin: string;
  /** Current URL without query or fragment. */
  href: string;
}

/** Bounded count values, omitting zero and unknown. The renderer draws attention as a dot
 * before the plain total and retains its count in the accessible description. */
export function entryCounts(item: { total?: number; count?: ShellCount }): ShellCount[] {
  return [
    ...(item.total && item.total > 0 ? [shellCount(item.total, 'plain')] : []),
    ...(item.count && item.count.text !== '0' ? [item.count] : []),
  ];
}

/** Count text is bounded at `99+`; the renderer uses attention as a dot and spoken count. */
export function shellCount(value: number, kind: 'needs-you' | 'plain' = 'needs-you'): ShellCount {
  return { text: countText(value), needsYou: kind === 'needs-you' };
}

/** The answer this browser read last, shared by every page so each paints the same rail first. */
const ANSWER_KEY = 'tmt-shell-answer-v1';
export function rememberedShellAnswer(): ShellAnswer | undefined {
  try {
    const stored = JSON.parse(sessionStorage.getItem(ANSWER_KEY) ?? 'null') as ShellAnswer | null;
    return stored?.version === 1 && Array.isArray(stored.modules) ? stored : undefined;
  } catch {
    return undefined;
  }
}

/** The door's mount of one module: `/r/<prefix>/x/<module>/`, the prefix being the machine's 16
 * lowercase base32 letters. The one definition of that shape for every product. */
/** A module mount, `/r/<prefix>/x/<module>/`, at the start of a path. */
const MODULE_MOUNT = /^(\/r\/[a-z2-7]{16})\/x\/([a-z0-9][a-z0-9-]{0,31})\//;

/** The module a mount path names, or undefined when `pathname` is not exactly a module mount. */
export function moduleMount(pathname: string): string | undefined {
  const mount = MODULE_MOUNT.exec(pathname);
  return mount?.[0] === pathname ? mount[2] : undefined;
}

/** The module whose mount `pathname` lies under, and the rest of the path below that mount. */
export function underModuleMount(pathname: string): { module: string; rest: string } | undefined {
  const mount = MODULE_MOUNT.exec(pathname);
  return mount ? { module: mount[2]!, rest: pathname.slice(mount[0].length) } : undefined;
}

/**
 * Where this page reads the composed shell: the `/r/<prefix>` mount it sits under, else the door's
 * answer for its path (`POST /sdk/mount`). Undefined when neither answers.
 */
export async function shellAddress(): Promise<string | undefined> {
  const mounted = MODULE_MOUNT.exec(location.pathname);
  if (mounted) return `${location.origin}${mounted[1]}`;
  try {
    const response = await fetch('/sdk/mount', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: location.pathname }),
    });
    if (response.status !== 200) return undefined;
    const { address } = (await response.json()) as { address?: unknown };
    return typeof address === 'string' ? address : undefined;
  } catch {
    return undefined;
  }
}

/** Fetch the owner's composed shell once per page; any failure leaves the remembered answer. */
export async function loadShellAnswer(address: string): Promise<ShellAnswer | undefined> {
  try {
    const response = await fetch(`${address}/x/-/shell`, {
      credentials: 'same-origin',
      cache: 'no-store',
    });
    if (!response.ok) return undefined;
    const answer = (await response.json()) as ShellAnswer;
    if (answer?.version !== 1 || !Array.isArray(answer.modules)) return undefined;
    try {
      sessionStorage.setItem(ANSWER_KEY, JSON.stringify(answer));
    } catch {
      // Without storage each page still reads its own answer.
    }
    return answer;
  } catch {
    return undefined;
  }
}

/**
 * The one rail (spec c5ccd17a): the same items in the same order on every page, whatever module
 * hosts it. A module the composed answer does not describe stays, dimmed, with the reason.
 */
const RAIL: readonly { id: string; label: string; icon: ShellIcon; path: string }[] = [
  { id: 'colab', label: 'Colab', icon: 'file-text', path: '/colab/' },
  { id: 'remote', label: 'Remote', icon: 'monitor-smartphone', path: '/remote/' },
  { id: 'ops', label: 'Ops', icon: 'users', path: '/ops/' },
  { id: 'digest', label: 'Digest', icon: 'chart-column', path: '/digest/' },
];
/** Served by the TMT host itself, so always reachable from the rail. */
const HOST_SERVED = new Set(['remote', 'ops', 'digest']);

/**
 * Home, the fixed modules (Colab, Remote, Ops, Digest), any other described module in Remote's
 * order, then TMT settings at the bottom. Colab is available
 * when the answer describes it or it hosts the page; without an answer (not paired) it is dimmed.
 * The current item is the host's module, else the longest matching path.
 */
export function shellRail(
  answer: ShellAnswer | undefined,
  place: ShellPlace,
  host?: ShellModule,
): ShellView['rail'] {
  const active = host?.extension;
  const described = new Map((answer?.modules ?? []).map((module) => [module.extension, module]));
  // The host page's own module is available even when the answer does not describe it.
  const hosted = (id: string) => host?.extension === id;
  const path = place.href.slice(place.origin.length);
  const settings = path === '/settings' || path.startsWith('/settings/');
  const unavailable = answer
    ? 'Not installed on this machine'
    : 'Pair this browser with Remote to open it';
  const modules = [
    ...RAIL.map((fixed) => {
      const module = described.get(fixed.id);
      return {
        id: fixed.id,
        label: module?.rail.label ?? fixed.label,
        icon: module ? moduleIcon(module.rail.icon) : fixed.icon,
        href:
          module?.rail.path ??
          (hosted(fixed.id) ? host!.rail.path : `${place.origin}${fixed.path}`),
        needsYou: module?.needsYou,
        available: Boolean(module) || hosted(fixed.id) || HOST_SERVED.has(fixed.id),
      };
    }),
    ...[...described.values(), ...(host && !described.has(host.extension) ? [host] : [])]
      .filter((module) => !RAIL.some((fixed) => fixed.id === module.extension))
      .sort((a, b) => a.rail.order - b.rail.order)
      .map((module) => ({
        id: module.extension,
        label: module.rail.label,
        icon: moduleIcon(module.rail.icon),
        href: module.rail.path,
        needsYou: module.needsYou,
        available: true,
      })),
  ];
  const matched =
    active ??
    modules
      .filter((module) => module.available && place.href.startsWith(module.href))
      .sort((a, b) => b.href.length - a.href.length)[0]?.id;
  const counts = [...described.values()].map((module) => module.needsYou);
  const total =
    answer && counts.every((count) => count !== undefined)
      ? counts.reduce<number>((sum, count) => sum + count!, 0)
      : undefined;
  const main: ShellItem[] = [
    {
      id: 'home',
      label: 'Home',
      icon: 'house',
      href: `${place.origin}/`,
      count: total ? shellCount(total) : undefined,
      current: path === '/' && !active,
    },
    ...modules.map((module) => ({
      id: module.id,
      label: module.label,
      icon: module.icon,
      href: module.href,
      count: module.needsYou ? shellCount(module.needsYou) : undefined,
      current: !settings && module.id === matched,
      ...(module.available ? {} : { disabled: true, note: unavailable }),
    })),
  ];
  const bottom: ShellItem[] = [
    {
      id: 'settings',
      label: 'TMT settings',
      icon: 'settings',
      href: `${place.origin}/settings/`,
      current: settings,
    },
  ];
  return { main, bottom };
}

/** A described module's own menu: entries in order, grouped, then its settings pages. */
export function describedMenu(module: ShellModule, place: ShellPlace): ShellMenu {
  const groups: ShellMenu['groups'] = [];
  for (const entry of module.menu) {
    const last = groups.at(-1);
    const item = {
      id: entry.id,
      label: entry.label,
      href: entry.path,
      count:
        entry.count && entry.count.value > 0
          ? shellCount(entry.count.value, entry.count.kind)
          : undefined,
      current: false,
    };
    if (last && last.label === entry.group) last.items.push(item);
    else groups.push({ label: entry.group, items: [item] });
  }
  if (module.settings.length)
    groups.push({
      items: module.settings.map((page) => ({
        id: page.id,
        label: page.label,
        href: page.path,
        current: false,
      })),
    });
  const items = groups.flatMap((group) => group.items);
  const current = items
    .filter((item) => place.href.startsWith(item.href))
    .sort((a, b) => b.href.length - a.href.length)[0];
  if (current) current.current = true;
  return { title: module.rail.label, groups };
}

const SVG = 'http://www.w3.org/2000/svg';

function icon(document: Document, name: ShellIcon): SVGSVGElement {
  const svg = document.createElementNS(SVG, 'svg');
  svg.setAttribute('class', 'tmt-ui-shell-icon');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('aria-hidden', 'true');
  for (const [tag, attributes] of shellIconNodes[name]) {
    const node = document.createElementNS(SVG, tag);
    for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
    svg.append(node);
  }
  return svg;
}
function count(document: Document, text: string): HTMLElement {
  const badge = document.createElement('span');
  badge.className = 'tmt-ui-shell-count';
  // The number alone: the colour and the item's accessible name say what it counts.
  badge.textContent = text;
  badge.setAttribute('aria-hidden', 'true');
  return badge;
}

function attention(document: Document): HTMLElement {
  const dot = document.createElement('span');
  dot.className = 'tmt-ui-attention-dot';
  dot.setAttribute('aria-hidden', 'true');
  return dot;
}

/** The bar's right-hand cluster: a module fills `search` and `chip`; an empty slot takes no room. */
export interface ShellTools {
  readonly search: HTMLElement;
  readonly chip: HTMLElement;
}

export interface ShellControls {
  readonly tools: ShellTools;
  /** Root-only common bindings; route modules never receive this method. */
  setCommonControls(options: ShellCommonControls | undefined): void;
  /** True while common controls are installed and the shell is not disposed. */
  topBarReady(): boolean;
  setTopBar(): ShellTopBar;
  /** The route owner releases the active action slot and transient common UI. */
  clearTopBar(): void;
  update(view: ShellView): void;
  dispose(): void;
}

/**
 * The one selected row a menu shows: the first current entry, where a current child selects
 * itself while its parent is expanded and selects its parent while collapsed.
 */
export function visibleCurrent(menu: ShellMenu | undefined): ShellMenuItem | undefined {
  if (!menu) return undefined;
  for (const item of [
    ...menu.groups.flatMap((group) => group.items),
    ...(menu.settings ? [menu.settings] : []),
  ]) {
    const child = item.children?.find((entry) => entry.current);
    if (child) return item.expanded ? child : item;
    if (item.current) return item;
  }
  return undefined;
}

let branchIds = 0;

/**
 * Render the rail and module menu into `root`, the `.tmt-ui-shell` element that holds
 * `.tmt-ui-shell-content` with the host's `.tmt-ui-shell-bar` (its one header) and
 * `.tmt-ui-shell-main`. On wide screens the TMT mark and the bar form one header row
 * (TMT, then the host's `<module> / <page>`), over the rail, the module menu column and the main
 * area; phones show the same nested tree inside the navigation drawer. `select` may take over a plain click
 * on an item (an SPA route or Colab's search); it returns true when it handled the click.
 */
export function mountShell(
  root: HTMLElement,
  view: ShellView,
  select?: (id: string, href: string) => boolean,
): ShellControls {
  const document = root.ownerDocument;
  root.classList.add('tmt-ui-shell');
  const open = document.createElement('button');
  open.type = 'button';
  open.className = 'tmt-ui-shell-open';
  open.setAttribute('aria-label', 'Open navigation');
  open.setAttribute('aria-expanded', 'false');
  open.append(icon(document, 'menu'));
  // The rail's head: the TMT mark and wordmark. It is the one place the brand sits; pages carry
  // no breadcrumb.
  const brand = document.createElement('div');
  brand.className = 'tmt-ui-shell-brand';
  // One rail per document draws its own paint servers: a second shell (a fixture, the gallery, a
  // hidden earlier mount) must not make the visible mark resolve to a hidden copy.
  const markId = freshMarkId(
    'tmt-rail',
    (prefix) => document.getElementById(`${prefix}-aurora`) !== null,
  );
  brand.innerHTML = brandMarkSvg({ size: 32, id: markId });
  const text = document.createElement('span');
  text.className = 'tmt-ui-shell-brand-text';
  const word = document.createElement('span');
  word.className = 'tmt-ui-shell-brand-word';
  word.textContent = 'tmt';
  const sub = document.createElement('span');
  sub.className = 'tmt-ui-shell-brand-sub';
  sub.textContent = 'terminal team';
  text.append(word, sub);
  brand.append(text);
  const rail = document.createElement('nav');
  rail.className = 'tmt-ui-rail';
  rail.setAttribute('aria-label', 'TMT');
  const syncRailScroll = () => rail.toggleAttribute('data-scrolled', rail.scrollTop > 0);
  rail.addEventListener('scroll', syncRailScroll, { passive: true });
  const menu = document.createElement('nav');
  menu.className = 'tmt-ui-module-menu';
  root.prepend(open, rail);
  const bar = root.querySelector(':scope > .tmt-ui-shell-content > .tmt-ui-shell-bar')!;
  const railSearch = document.createElement('div');
  railSearch.className = 'tmt-ui-rail-search-section';
  const topBar = createShellTopBar(root, bar, open, railSearch);
  bar.after(menu);
  // The cluster is the bar's second layer: it sits over the bar's right end and tells the host
  // header how much room to leave (`--tmt-ui-shell-tools-width`). A React shell draws the element
  // itself, so its modules can portal into the slots on their first render.
  const slotOf = (parent: HTMLElement, name: 'search' | 'chip') => {
    const existing = parent.querySelector<HTMLElement>(`:scope > .tmt-ui-shell-tools-${name}`);
    if (existing) return existing;
    const created = document.createElement('div');
    created.className = `tmt-ui-shell-tools-${name}`;
    parent.append(created);
    return created;
  };
  const drawn = root.querySelector<HTMLElement>(
    ':scope > .tmt-ui-shell-content > .tmt-ui-shell-tools',
  );
  const toolsRoot = drawn ?? document.createElement('div');
  if (!drawn) {
    toolsRoot.className = 'tmt-ui-shell-tools';
    bar.after(toolsRoot);
  }
  const tools: ShellTools = {
    search: slotOf(toolsRoot, 'search'),
    chip: slotOf(toolsRoot, 'chip'),
  };
  const reserve = () => {
    // An empty cluster reserves nothing: the stylesheet's 0px default stands and the root keeps no
    // inline style.
    const width = toolsRoot.offsetWidth;
    if (width > 0) root.style.setProperty('--tmt-ui-shell-tools-width', `${width}px`);
    else root.style.removeProperty('--tmt-ui-shell-tools-width');
  };
  const watchTools = typeof ResizeObserver === 'function' ? new ResizeObserver(reserve) : undefined;
  watchTools?.observe(toolsRoot);
  reserve();

  const setDrawer = (shown: boolean) => {
    root.dataset.drawer = shown ? 'open' : 'closed';
    open.setAttribute('aria-expanded', String(shown));
  };
  // Unavailable items keep their reason beside the hovered or focused item, in the top layer.
  const tip = document.createElement('div');
  tip.className = 'tmt-ui-rail-tip';
  tip.setAttribute('role', 'tooltip');
  tip.popover = 'manual';
  root.append(tip);
  let release = () => {};
  const hideTip = () => {
    release();
    if (tip.matches(':popover-open')) tip.hidePopover();
  };
  const showTip = (anchor: HTMLElement) => {
    const text = anchor.dataset.note;
    if (!text || root.dataset.drawer === 'open') return hideTip();
    release();
    release = claimIconActionTooltip(document, hideTip);
    tip.textContent = text;
    if (!tip.matches(':popover-open')) tip.showPopover();
    const box = anchor.getBoundingClientRect();
    tip.style.left = `${box.right}px`;
    tip.style.top = `${box.top + (box.height - tip.offsetHeight) / 2}px`;
  };
  const tipFor = (anchor: HTMLElement, text: string) => {
    anchor.dataset.note = text;
    anchor.addEventListener('pointerenter', () => showTip(anchor));
    anchor.addEventListener('focus', () => showTip(anchor));
    anchor.addEventListener('pointerleave', hideTip);
    anchor.addEventListener('blur', hideTip);
  };
  open.onclick = () => setDrawer(root.dataset.drawer !== 'open');
  const close = (event: KeyboardEvent) => {
    if (event.key === 'Escape' && root.dataset.drawer === 'open') {
      setDrawer(false);
      open.focus();
    }
  };
  document.addEventListener('keydown', close);
  // A press outside the open drawer closes it, as an overlay does.
  const outside = (event: PointerEvent) => {
    const target = event.target as Node;
    if (root.dataset.drawer === 'open' && !rail.contains(target) && !open.contains(target))
      setDrawer(false);
  };
  document.addEventListener('pointerdown', outside);

  const link = (className: string, item: ShellItem | ShellMenuItem, selected: boolean) => {
    const anchor = document.createElement('a');
    anchor.className = className;
    anchor.href = item.href;
    anchor.dataset.id = item.id;
    if (selected) anchor.setAttribute('aria-current', 'page');
    const disabled = 'disabled' in item && item.disabled;
    if (disabled) anchor.setAttribute('aria-disabled', 'true');
    anchor.onclick = (event) => {
      if (disabled) return event.preventDefault();
      if (event.button || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
      setDrawer(false);
      if (select?.(item.id, item.href)) event.preventDefault();
    };
    return anchor;
  };
  // One child tree serves the wide rail and phone drawer.
  const menuEntry = (
    className: string,
    item: ShellMenuItem,
    selected: ShellMenuItem | undefined,
  ) => {
    const anchor = link(className, item, item === selected);
    if (item.note) anchor.title = item.note;
    const label = document.createElement('span');
    label.className = 'tmt-ui-module-menu-label';
    label.textContent = item.label;
    if (!item.note) label.title = item.label;
    if (item.icon) anchor.append(icon(document, item.icon));
    anchor.append(label);
    if (item.emphasis === 'unread') {
      anchor.dataset.emphasis = 'unread';
      const spoken = document.createElement('span');
      spoken.className = 'tmt-ui-visually-hidden';
      spoken.textContent = ' unread';
      anchor.append(spoken);
    }
    const shown = entryCounts(item);
    const total = shown.find((value) => !value.needsYou);
    const waiting = shown.find((value) => value.needsYou);
    if (shown.length || item.actions?.length) {
      const counts = document.createElement('span');
      counts.className = 'tmt-ui-shell-counts';
      if (waiting) counts.append(attention(document));
      if (total) counts.append(count(document, total.text));
      anchor.append(counts);
    }
    if (shown.length) {
      const summary = [
        total && `${item.total ?? total.text} total`,
        waiting && `${waiting.text} need you`,
      ]
        .filter(Boolean)
        .join(', ');
      anchor.setAttribute(
        'aria-label',
        `${item.label}${item.emphasis === 'unread' ? ' unread' : ''} · ${summary}`,
      );
      anchor.title = item.note ? `${summary} · ${item.note}` : summary;
      tipFor(anchor, `${item.label} · ${anchor.title}`);
    }
    return anchor;
  };
  const command = (
    className: string,
    action: ShellMenuAction,
    activate: () => void,
    mark?: ShellIcon,
  ) => {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = className;
    button.title = action.label;
    if (mark) {
      button.setAttribute('aria-label', action.label);
      button.append(icon(document, mark));
      tipFor(button, action.label);
    } else button.textContent = action.label;
    button.onclick = () => {
      setDrawer(false);
      activate();
    };
    return button;
  };
  const announcement = document.createElement('div');
  announcement.className = 'tmt-ui-visually-hidden';
  announcement.setAttribute('role', 'status');
  announcement.setAttribute('aria-live', 'polite');
  announcement.setAttribute('aria-atomic', 'true');
  let menuInteractions: ShellMenuInteractions[] = [];
  const interactions = (current: () => ShellMenuGroup | undefined) => {
    const controls = createShellMenuInteractions(
      current,
      (name) => icon(document, name),
      (action) => {
        setDrawer(false);
        action.onActivate();
      },
      (message) => {
        announcement.textContent = message;
      },
    );
    menuInteractions.push(controls);
    return controls;
  };
  // A parent row: its link, a disclosure beside it and, while expanded, one level of children.
  const disclose = (
    row: HTMLLIElement,
    anchor: HTMLAnchorElement,
    item: ShellMenuItem,
    index: number,
    selected: ShellMenuItem | undefined,
  ) => {
    const live = () => latest.menu?.groups[index]?.items.find((entry) => entry.id === item.id);
    const expanded = item.expanded === true;
    row.classList.add('tmt-ui-rail-menu-row');
    row.dataset.menuItem = item.id;
    row.dataset.disclosure = expanded ? 'open' : 'closed';
    anchor.dataset.railPart = 'link';
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'tmt-ui-rail-group-action tmt-ui-rail-disclosure';
    button.dataset.railPart = 'disclosure';
    button.setAttribute('aria-expanded', String(expanded));
    button.setAttribute('aria-label', `${expanded ? 'Hide' : 'Show'} ${item.label}`);
    button.append(icon(document, 'chevron-right'));
    // Opening or closing a branch is not navigation: the phone drawer stays open.
    button.onclick = () => live()?.toggle?.();
    row.append(button);
    if (!expanded) return;
    const children = document.createElement('ul');
    children.className = 'tmt-ui-rail-sub-children';
    children.id = `tmt-ui-rail-children-${++branchIds}`;
    children.setAttribute('aria-label', item.label);
    button.setAttribute('aria-controls', children.id);
    for (const child of item.children!) {
      const childRow = document.createElement('li');
      const link = menuEntry('tmt-ui-module-menu-item tmt-ui-rail-sub-item', child, selected);
      childRow.classList.add('tmt-ui-rail-menu-row');
      childRow.dataset.menuItem = child.id;
      childRow.dataset.menuGroup = String(index);
      link.dataset.railPart = 'link';
      childRow.append(link);
      if (child.actions?.length)
        interactions(() => {
          const branch = live()?.children;
          return branch ? { items: [...branch] } : undefined;
        }).decorate(childRow, link, child);
      children.append(childRow);
    }
    row.append(children);
  };
  // Up/Down walk the visible rows; Right/Left open, enter, close or leave a branch. Alt+Up/Down
  // stays the reorder request.
  const walk = (list: HTMLUListElement) =>
    list.addEventListener('keydown', (event) => {
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
      const row = (event.target as HTMLElement).closest('li');
      const link = row?.querySelector<HTMLAnchorElement>(':scope > a');
      if (!row || !link) return;
      const disclosure = row.querySelector<HTMLButtonElement>(':scope > .tmt-ui-rail-disclosure');
      const open = disclosure?.getAttribute('aria-expanded') === 'true';
      const branch = row.parentElement?.classList.contains('tmt-ui-rail-sub-children')
        ? row.parentElement.parentElement?.querySelector<HTMLAnchorElement>(':scope > a')
        : undefined;
      let next: HTMLElement | null | undefined;
      if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
        const links = [...list.querySelectorAll<HTMLAnchorElement>('a.tmt-ui-rail-sub-item')];
        next = links[links.indexOf(link) + (event.key === 'ArrowUp' ? -1 : 1)];
        event.preventDefault();
      } else if (event.key === 'ArrowRight' && disclosure) {
        event.preventDefault();
        if (open) next = row.querySelector<HTMLAnchorElement>(':scope > ul a');
        else disclosure.click();
      } else if (event.key === 'ArrowLeft' && (open || branch)) {
        event.preventDefault();
        if (open) disclosure!.click();
        else next = branch;
      }
      next?.focus();
    });
  const nested = (menuView: ShellMenu, selected: ShellMenuItem | undefined) => {
    const list = document.createElement('ul');
    list.className = 'tmt-ui-rail-sub';
    list.setAttribute('aria-label', menuView.title);
    walk(list);
    menuView.groups.forEach((group, index) => {
      const controls =
        group.reorder || group.items.some((item) => item.actions?.length)
          ? interactions(() => latest.menu?.groups[index])
          : undefined;
      let entries = list;
      if (group.label || group.action) {
        const section = document.createElement('li');
        const heading = document.createElement('div');
        heading.className = 'tmt-ui-rail-sub-group';
        const label = document.createElement('span');
        label.textContent = group.label ?? '';
        label.title = group.label ?? '';
        heading.append(label);
        if (group.action)
          heading.append(
            command(
              `tmt-ui-rail-group-action${group.action.icon ? '' : ' tmt-ui-rail-group-text-action'}`,
              group.action,
              () => latest.menu?.groups[index]?.action?.onActivate(),
              group.action.icon,
            ),
          );
        entries = document.createElement('ul');
        entries.className = 'tmt-ui-rail-sub-items';
        if (group.label) entries.setAttribute('aria-label', group.label);
        section.append(heading, entries);
        list.append(section);
      }
      for (const item of group.items) {
        const row = document.createElement('li');
        const anchor = menuEntry('tmt-ui-module-menu-item tmt-ui-rail-sub-item', item, selected);
        row.dataset.menuGroup = String(index);
        row.append(anchor);
        if (item.actions?.length || group.reorder) controls?.decorate(row, anchor, item);
        if (item.children?.length) disclose(row, anchor, item, index, selected);
        entries.append(row);
      }
      if (group.add) {
        const row = document.createElement('li');
        const add = command('tmt-ui-rail-sub-item tmt-ui-rail-add', group.add, () =>
          latest.menu?.groups[index]?.add?.onActivate(),
        );
        add.prepend(icon(document, 'plus'));
        row.append(add);
        entries.append(row);
      }
    });
    if (menuView.settings) {
      const row = document.createElement('li');
      row.className = 'tmt-ui-rail-settings';
      const anchor = menuEntry(
        'tmt-ui-module-menu-item tmt-ui-rail-sub-item',
        menuView.settings,
        selected,
      );
      row.dataset.menuGroup = 'settings';
      row.append(anchor);
      if (menuView.settings.actions?.length)
        interactions(() =>
          latest.menu?.settings ? { items: [latest.menu.settings] } : undefined,
        ).decorate(row, anchor, menuView.settings);
      list.append(row);
    }
    return list;
  };
  const railList = (
    items: ShellItem[],
    current: ShellItem | undefined,
    selected: ShellMenuItem | undefined,
    extra?: string,
    pages?: ShellMenu,
  ) => {
    const list = document.createElement('ul');
    list.className = extra ? `tmt-ui-rail-items ${extra}` : 'tmt-ui-rail-items';
    for (const item of items) {
      const anchor = link('tmt-ui-rail-item', item, item === current && !selected);
      if (item === current) anchor.dataset.moduleCurrent = '';
      const badge = item.count?.needsYou && item.count.text !== '0' ? item.count : undefined;
      if (badge)
        anchor.setAttribute(
          'aria-label',
          `${item.label}, ${badge.text}${badge.text === '1' ? ' needs you' : ' need you'}`,
        );
      anchor.title = item.note ?? item.label;
      if (item.note && !badge) tipFor(anchor, `${item.label}: ${item.note}`);
      const label = document.createElement('span');
      label.className = 'tmt-ui-rail-label';
      label.textContent = item.label;
      anchor.append(icon(document, item.icon), label);
      if (badge) {
        anchor.append(attention(document));
        tipFor(anchor, `${item.label}, ${badge.text} need you${item.note ? `: ${item.note}` : ''}`);
      }
      const row = document.createElement('li');
      row.append(anchor);
      if (item === current && pages) row.append(nested(pages, selected));
      list.append(row);
    }
    return list;
  };

  // Hosts repaint on every data refresh; an unchanged view keeps its nodes, so a press in
  // progress and keyboard focus survive.
  let shown = '';
  let latest = view;
  const render = (next: ShellView) => {
    latest = next;
    // Callback identity is not view state, but adding/removing a reorder affordance is.
    const key = JSON.stringify([next, next.menu?.groups.map((group) => Boolean(group.reorder))]);
    if (key === shown) return;
    shown = key;
    const focused = rail.contains(document.activeElement)
      ? (document.activeElement as HTMLElement)
      : undefined;
    const owner = focused?.closest<HTMLElement>('[data-menu-item]');
    const focus = owner
      ? {
          id: owner.dataset.menuItem,
          group: owner.dataset.menuGroup,
          part: focused?.dataset.railPart,
        }
      : undefined;
    for (const controls of menuInteractions) controls.dispose();
    menuInteractions = [];
    hideTip();
    const railTop = rail.scrollTop;
    const current = [...next.rail.main, ...next.rail.bottom].find((item) => item.current);
    const selected = current ? visibleCurrent(next.menu) : undefined;
    rail.replaceChildren(
      brand,
      railSearch,
      railList(next.rail.main, current, selected, undefined, next.menu),
      railList(next.rail.bottom, current, selected, 'tmt-ui-rail-bottom', next.menu),
    );
    // Keep the same scroll owner across refreshes; shrinking content clamps its position.
    rail.scrollTop = railTop;
    syncRailScroll();
    root.dataset.menu = next.menu ? 'shown' : 'none';
    menu.replaceChildren();
    if (next.menu?.groups.some((group) => group.reorder)) {
      if (!announcement.isConnected) root.append(announcement);
    } else announcement.remove();
    if (focus) {
      const restored = [...rail.querySelectorAll<HTMLElement>('[data-rail-part]')].find(
        (element) => {
          const row = element.closest<HTMLElement>('[data-menu-item]');
          return (
            row !== null &&
            row.dataset.menuItem === focus.id &&
            row.dataset.menuGroup === focus.group &&
            element.dataset.railPart === focus.part
          );
        },
      );
      if (restored && restored.getClientRects().length) {
        restored.focus({ preventScroll: true });
        restored.scrollIntoView({ block: 'nearest' });
      }
    }
  };
  setDrawer(false);
  render(view);
  return {
    tools,
    setCommonControls: topBar.setCommonControls,
    topBarReady: topBar.topBarReady,
    setTopBar: topBar.setTopBar,
    clearTopBar: topBar.clear,
    update: render,
    dispose() {
      for (const controls of menuInteractions) controls.dispose();
      menuInteractions = [];
      announcement.remove();
      topBar.dispose();
      watchTools?.disconnect();
      root.style.removeProperty('--tmt-ui-shell-tools-width');
      if (!drawn) toolsRoot.remove();
      document.removeEventListener('keydown', close);
      document.removeEventListener('pointerdown', outside);
      hideTip();
      open.remove();
      tip.remove();
      rail.removeEventListener('scroll', syncRailScroll);
      rail.remove();
      menu.remove();
    },
  };
}
