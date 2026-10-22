import { brandMarkSvg } from './brand-mark-svg.js';
import { claimIconActionTooltip } from './icon-action-tooltip.js';
import { moduleIcon, shellIconNodes, type ShellIcon } from './shell-icons.js';

/**
 * The one TMT web shell (#3937, Colab page c5ccd17a): a global rail that collapses to icons,
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
  /** A plain total shown before a needs-you count, as in `6 · ◆ 1`. */
  total?: number;
  /** Shown but not followed; `note` says why (its tooltip and description). */
  disabled?: boolean;
  note?: string;
}
export interface ShellMenu {
  title: string;
  groups: { label?: string; items: ShellMenuItem[] }[];
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

/** Counts above 99 read `99+`; needs-you counts carry the ◆ mark and colour. */
export function shellCount(value: number, kind: 'needs-you' | 'plain' = 'needs-you'): ShellCount {
  return { text: value > 99 ? '99+' : String(value), needsYou: kind === 'needs-you' };
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
      count: entry.count ? shellCount(entry.count.value, entry.count.kind) : undefined,
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

const RAIL_KEY = 'tmt.shell.rail';
const SVG = 'http://www.w3.org/2000/svg';

function railCollapsed(): boolean {
  try {
    return localStorage.getItem(RAIL_KEY) === 'collapsed';
  } catch {
    return false;
  }
}
function rememberRail(collapsed: boolean): void {
  try {
    localStorage.setItem(RAIL_KEY, collapsed ? 'collapsed' : 'expanded');
  } catch {
    // The choice still applies to this page.
  }
}

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
function count(document: Document, value: ShellCount): HTMLElement {
  const badge = document.createElement('span');
  badge.className = 'tmt-ui-shell-count';
  if (value.needsYou) badge.dataset.kind = 'needs-you';
  badge.textContent = value.needsYou ? `◆ ${value.text}` : value.text;
  return badge;
}

export interface ShellControls {
  update(view: ShellView): void;
  dispose(): void;
}

/**
 * Render the rail and module menu into `root`, the `.tmt-ui-shell` element that holds
 * `.tmt-ui-shell-content` with the host's `.tmt-ui-shell-bar` (its one header) and
 * `.tmt-ui-shell-main`. On wide screens the TMT mark and the bar form one header row
 * (TMT, then the host's `<module> / <page>`), over the rail, the module menu column and the main
 * area; on phones the module menu becomes tabs under the title bar. `select` may take over a plain click
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
  const brand = document.createElement('span');
  brand.className = 'tmt-ui-shell-brand';
  brand.innerHTML = brandMarkSvg({ size: 24, id: 'tmt-rail' });
  const word = document.createElement('span');
  word.className = 'tmt-ui-shell-brand-word';
  word.textContent = 'TMT';
  // One breadcrumb: TMT ・ <module> / <page>.
  const dot = document.createElement('span');
  dot.className = 'tmt-ui-shell-brand-separator';
  dot.setAttribute('aria-hidden', 'true');
  dot.textContent = '·';
  brand.append(word, dot);
  // The host's header starts where the mark ends, so the two read as one breadcrumb. The width
  // goes into an adopted sheet, not a style attribute: pages keep no inline styles.
  const sheet =
    typeof ResizeObserver === 'function' && 'adoptedStyleSheets' in document
      ? new CSSStyleSheet()
      : undefined;
  if (sheet) document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
  const measure = sheet
    ? new ResizeObserver(() =>
        sheet.replaceSync(`.tmt-ui-shell { --tmt-ui-shell-brand-width: ${brand.offsetWidth}px; }`),
      )
    : undefined;
  measure?.observe(brand);
  const rail = document.createElement('nav');
  rail.className = 'tmt-ui-rail';
  rail.setAttribute('aria-label', 'TMT');
  const menu = document.createElement('nav');
  menu.className = 'tmt-ui-module-menu';
  root.prepend(open, brand, rail);
  root.querySelector(':scope > .tmt-ui-shell-content > .tmt-ui-shell-bar')!.after(menu);

  let collapsed = railCollapsed();
  const setDrawer = (shown: boolean) => {
    root.dataset.drawer = shown ? 'open' : 'closed';
    open.setAttribute('aria-expanded', String(shown));
  };
  // Labels on the collapsed rail (and an unavailable item's reason) show as one tooltip beside
  // the hovered or focused item, in the top layer.
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
    const text = collapsed ? anchor.dataset.tip : anchor.dataset.note;
    if (!text || root.dataset.drawer === 'open') return hideTip();
    release();
    release = claimIconActionTooltip(document, hideTip);
    tip.textContent = text;
    if (!tip.matches(':popover-open')) tip.showPopover();
    const box = anchor.getBoundingClientRect();
    tip.style.left = `${box.right}px`;
    tip.style.top = `${box.top + (box.height - tip.offsetHeight) / 2}px`;
  };
  const tipFor = (anchor: HTMLElement, text: string, note?: string) => {
    anchor.dataset.tip = text;
    if (note) anchor.dataset.note = text;
    anchor.addEventListener('pointerenter', () => showTip(anchor));
    anchor.addEventListener('focus', () => showTip(anchor));
    anchor.addEventListener('pointerleave', hideTip);
    anchor.addEventListener('blur', hideTip);
  };
  const paint = () => {
    root.dataset.rail = collapsed ? 'collapsed' : 'expanded';
    const label = collapsed ? 'Expand sidebar' : 'Collapse sidebar';
    toggle.setAttribute('aria-label', label);
    toggle.setAttribute('aria-expanded', String(!collapsed));
    toggle.dataset.tip = label;
    toggle.replaceChildren(icon(document, collapsed ? 'panel-left-open' : 'panel-left-close'));
  };
  // The toggle has its own row above the rail's items.
  const toggle = document.createElement('button');
  toggle.type = 'button';
  toggle.className = 'tmt-ui-rail-toggle';
  tipFor(toggle, '');
  toggle.onclick = () => {
    collapsed = !collapsed;
    rememberRail(collapsed);
    paint();
    hideTip();
  };
  const top = document.createElement('div');
  top.className = 'tmt-ui-rail-top';
  top.append(toggle);
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

  const link = (className: string, item: ShellItem | ShellMenuItem) => {
    const anchor = document.createElement('a');
    anchor.className = className;
    anchor.href = item.href;
    anchor.dataset.id = item.id;
    if (item.current) anchor.setAttribute('aria-current', 'page');
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
  const railList = (items: ShellItem[], extra?: string) => {
    const list = document.createElement('ul');
    list.className = extra ? `tmt-ui-rail-items ${extra}` : 'tmt-ui-rail-items';
    for (const item of items) {
      const anchor = link('tmt-ui-rail-item', item);
      tipFor(anchor, item.note ? `${item.label}: ${item.note}` : item.label, item.note);
      const label = document.createElement('span');
      label.className = 'tmt-ui-rail-label';
      label.textContent = item.label;
      anchor.append(icon(document, item.icon), label);
      if (item.count) anchor.append(count(document, item.count));
      const row = document.createElement('li');
      row.append(anchor);
      list.append(row);
    }
    return list;
  };

  // Hosts repaint on every data refresh; an unchanged view keeps its nodes, so a press in
  // progress and keyboard focus survive.
  let shown = '';
  const render = (next: ShellView) => {
    const key = JSON.stringify(next);
    if (key === shown) return;
    shown = key;
    hideTip();
    rail.replaceChildren(
      top,
      railList(next.rail.main),
      railList(next.rail.bottom, 'tmt-ui-rail-bottom'),
    );
    root.dataset.menu = next.menu ? 'shown' : 'none';
    if (!next.menu) {
      menu.replaceChildren();
      menu.removeAttribute('aria-label');
      return;
    }
    menu.setAttribute('aria-label', next.menu.title);
    // A phone's tab strip keeps its scroll across a rebuild, then shows the current tab.
    const left = menu.scrollLeft;
    menu.replaceChildren();
    for (const group of next.menu.groups) {
      if (group.label !== undefined || menu.childElementCount > 0) {
        const heading = document.createElement('div');
        heading.className = 'tmt-ui-module-menu-group';
        heading.textContent = group.label ?? '';
        menu.append(heading);
      }
      const list = document.createElement('ul');
      list.className = 'tmt-ui-module-menu-items';
      for (const item of group.items) {
        const anchor = link('tmt-ui-module-menu-item', item);
        if (item.note) anchor.title = item.note;
        const label = document.createElement('span');
        label.className = 'tmt-ui-module-menu-label';
        label.textContent = item.label;
        if (!item.note) label.title = item.label;
        anchor.append(label);
        if (item.total !== undefined || item.count) {
          const counts = document.createElement('span');
          counts.className = 'tmt-ui-shell-counts';
          if (item.total !== undefined)
            counts.append(count(document, shellCount(item.total, 'plain')));
          if (item.count) counts.append(count(document, item.count));
          anchor.append(counts);
        }
        const row = document.createElement('li');
        row.append(anchor);
        list.append(row);
      }
      menu.append(list);
    }
    menu.scrollLeft = left;
    const current = menu.querySelector<HTMLElement>('[aria-current="page"]');
    if (current && menu.scrollWidth > menu.clientWidth) {
      const strip = menu.getBoundingClientRect();
      const box = current.getBoundingClientRect();
      if (box.left < strip.left) menu.scrollLeft -= strip.left - box.left;
      else if (box.right > strip.right) menu.scrollLeft += box.right - strip.right;
    }
  };
  paint();
  setDrawer(false);
  render(view);
  return {
    update: render,
    dispose() {
      document.removeEventListener('keydown', close);
      document.removeEventListener('pointerdown', outside);
      hideTip();
      measure?.disconnect();
      if (sheet)
        document.adoptedStyleSheets = document.adoptedStyleSheets.filter((item) => item !== sheet);
      open.remove();
      brand.remove();
      tip.remove();
      rail.remove();
      menu.remove();
    },
  };
}
