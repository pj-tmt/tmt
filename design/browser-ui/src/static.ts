import { shellIconNodes, type ShellIcon } from './shell-icons.js';
export const browserUiClasses = Object.freeze({
  chips: 'tmt-ui-chips',
  chip: 'tmt-ui-chip',
  chipLabel: 'tmt-ui-chip-label',
  chipsAdd: 'tmt-ui-chips-add',
  chipsStatus: 'tmt-ui-chips-status',
  chipsSurface: 'tmt-ui-chips-surface',
  chipsSearch: 'tmt-ui-chips-search',
  chipsOption: 'tmt-ui-chips-option',
  countChip: 'tmt-ui-count-chip',
  count: 'tmt-ui-count',
  avatar: 'tmt-ui-avatar',
  radio: 'tmt-ui-radio',
  choices: 'tmt-ui-choices',
  choicesLegend: 'tmt-ui-choices-legend',
  choice: 'tmt-ui-choice',
  choiceText: 'tmt-ui-choice-text',
  choiceDescription: 'tmt-ui-choice-description',
  choiceHint: 'tmt-ui-choice-hint',
  pager: 'tmt-ui-pager',
  pagerPosition: 'tmt-ui-pager-position',
  statusTiles: 'tmt-ui-status-tiles',
  statusTile: 'tmt-ui-status-tile',
  statusTileLabel: 'tmt-ui-status-tile-label',
  statusTileValue: 'tmt-ui-status-tile-value',
  statusTileDot: 'tmt-ui-status-tile-dot',
  stateMark: 'tmt-ui-state-mark',
  card: 'tmt-ui-card',
  cardHead: 'tmt-ui-card-head',
  cardTitle: 'tmt-ui-card-title',
  cardMeta: 'tmt-ui-card-meta',
  meter: 'tmt-ui-meter',
  meterFill: 'tmt-ui-meter-fill',
  glyph: 'tmt-ui-glyph',
  pageHead: 'tmt-ui-page-head',
  pageTitle: 'tmt-ui-page-title',
  pageSubtitle: 'tmt-ui-page-subtitle',
  stats: 'tmt-ui-stats',
  stat: 'tmt-ui-stat',
  statValue: 'tmt-ui-stat-value',
  statNumber: 'tmt-ui-stat-number',
  statCaption: 'tmt-ui-stat-caption',
  banner: 'tmt-ui-banner',
  bannerHead: 'tmt-ui-banner-head',
  bannerRow: 'tmt-ui-banner-row',
  facts: 'tmt-ui-facts',
  tableScroll: 'tmt-ui-table-scroll',
  tableGroup: 'tmt-ui-table-group',
  tableGroupToggle: 'tmt-ui-table-group-toggle',
  tableGroupCount: 'tmt-ui-table-group-count',
  tableSelect: 'tmt-ui-table-select',

  notifications: 'tmt-ui-notifications',
  notification: 'tmt-ui-notification',
  notificationStatus: 'tmt-ui-notification-status',
  notificationHeading: 'tmt-ui-notification-heading',
  notificationTag: 'tmt-ui-notification-tag',
  notificationLine: 'tmt-ui-notification-line',
  notificationQuote: 'tmt-ui-notification-quote',
  notificationWaiting: 'tmt-ui-notification-waiting',
  drawer: 'tmt-ui-drawer',
  drawerHeader: 'tmt-ui-drawer-header',
  drawerBody: 'tmt-ui-drawer-body',
  dialog: 'tmt-ui-dialog',
  dialogHeader: 'tmt-ui-dialog-header',
  dialogBody: 'tmt-ui-dialog-body',
  dialogActions: 'tmt-ui-dialog-actions',
  menu: 'tmt-ui-menu',
  menuSurface: 'tmt-ui-menu-surface',
  menuItem: 'tmt-ui-menu-item',
  command: 'tmt-ui-command',
  commandText: 'tmt-ui-command-text',
  commandCopy: 'tmt-ui-command-copy',
  commandFeedback: 'tmt-ui-command-feedback',
  code: 'tmt-ui-code',
  header: 'tmt-ui-header',
  brand: 'tmt-ui-brand',
  mark: 'tmt-ui-mark',
  wordmark: 'tmt-ui-wordmark',
  heading: 'tmt-ui-heading',
  headingSuffix: 'tmt-ui-heading-suffix',
  title: 'tmt-ui-title',
  caption: 'tmt-ui-caption',
  status: 'tmt-ui-status',
  actions: 'tmt-ui-actions',
  icon: 'tmt-ui-icon',
  notice: 'tmt-ui-notice',
  noticeMark: 'tmt-ui-notice-mark',
  noticeEyebrow: 'tmt-ui-notice-eyebrow',
  noticeHeading: 'tmt-ui-notice-heading',
  noticeBody: 'tmt-ui-notice-body',
  noticeActions: 'tmt-ui-notice-actions',
  field: 'tmt-ui-field',
  fieldLabel: 'tmt-ui-field-label',
  fieldControl: 'tmt-ui-field-control',
  searchField: 'tmt-ui-search-field',
  fieldDescription: 'tmt-ui-field-description',
  fieldError: 'tmt-ui-field-error',
  action: 'tmt-ui-action',
  actionLabel: 'tmt-ui-action-label',
  actionMark: 'tmt-ui-action-mark',
  busyMark: 'tmt-ui-busy-mark',
  busyDot: 'tmt-ui-busy-dot',
  checkbox: 'tmt-ui-checkbox',
  checkboxLabel: 'tmt-ui-checkbox-label',
  iconAction: 'tmt-ui-icon-action',
  iconActionControl: 'tmt-ui-icon-action-control',
  iconActionIcon: 'tmt-ui-icon-action-icon',
  iconActionTooltip: 'tmt-ui-icon-action-tooltip',
  iconActionTooltipLabel: 'tmt-ui-icon-action-tooltip-label',
  toggle: 'tmt-ui-toggle',
  toggleIndicator: 'tmt-ui-toggle-indicator',
  list: 'tmt-ui-list',
  listRow: 'tmt-ui-list-row',
  listLeading: 'tmt-ui-list-leading',
  listTitle: 'tmt-ui-list-title',
  listMeta: 'tmt-ui-list-meta',
  listLabel: 'tmt-ui-list-label',
  listDescription: 'tmt-ui-list-description',
  listUpdated: 'tmt-ui-list-updated',
  listState: 'tmt-ui-list-state',
  listActions: 'tmt-ui-list-actions',
  tabs: 'tmt-ui-tabs',
  tabsHeader: 'tmt-ui-tabs-header',
  tabList: 'tmt-ui-tab-list',
  tab: 'tmt-ui-tab',
  tabMark: 'tmt-ui-tab-mark',
  tabLabel: 'tmt-ui-tab-label',
  tabCount: 'tmt-ui-tab-count',
  tabsStatus: 'tmt-ui-tabs-status',
  tabPanel: 'tmt-ui-tab-panel',
  section: 'tmt-ui-section',
  sectionHeading: 'tmt-ui-section-heading',
  table: 'tmt-ui-table',
  disclosure: 'tmt-ui-disclosure',
  visuallyHidden: 'tmt-ui-visually-hidden',
} as const);

/** What a count chip draws: `attention` is for work that waits on the reader (Needs you). */
export type BrowserCountTone = 'neutral' | 'attention';
/** A count as drawn in a chip or the rail: counts above 99 read `99+`. */
export function countText(value: number): string {
  return value > 99 ? '99+' : String(value);
}

/** The initials an avatar draws: the first letters of up to two words of a name or handle. */
export function avatarInitials(name: string): string {
  const words = name.split(/[\s\-_.@]+/u).filter(Boolean);
  return words
    .slice(0, 2)
    .map((word) => [...word][0])
    .join('')
    .toLocaleUpperCase();
}

export type BrowserNoticeTone = 'waiting' | 'blocked' | 'working' | 'review' | 'muted';
export interface BrowserTab {
  id: string;
  tabId: string;
  panelId: string;
  label: string;
  /** A small muted number after the label (`99+` above 99), such as how many pages a state holds. */
  count?: number;
  state?: { mark: string; word: string; tone: BrowserNoticeTone };
}
export type BrowserTabSelectReason = 'keyboard' | 'activation';
/** A status tile's dot: the notice tones plus `info` for a read-only fact. */
export type BrowserStatusTone = BrowserNoticeTone | 'info';
export type BrowserActionVariant = 'text' | 'primary' | 'secondary' | 'destructive';
export type BrowserAnnouncement = 'status' | 'alert' | 'none';
export interface BrowserNotificationData {
  id: string;
  destination: 'Chat' | 'Thread' | 'Answer' | 'Proposal';
  firstLine: string;
  quote?: string;
}

export interface BrowserChipItem {
  id: string;
  label: string;
  removeLabel: string;
  disabled?: boolean;
}
export interface BrowserChipOption {
  id: string;
  label: string;
  description?: string;
  disabled?: boolean;
}

/**
 * The busy mark (Ben's pick a2) as text: a 2x3 braille cell whose three lit dots go round its
 * six-dot ring, clockwise from the top left. The browser mark draws the same snake as SVG dots
 * (`busyDots`); these frames are its text and TUI parity and the fallback where no SVG is drawn.
 */
export const busyFrames = ['⠙', '⠸', '⠴', '⠦', '⠇', '⠋'] as const;

/**
 * The browser busy mark's six dots on a 16x16 viewBox, in ring order (clockwise from the top
 * left of the 2x3 cell). Each dot's eased lap is offset by its place in the ring in CSS.
 */
export const busyDots: readonly (readonly [cx: number, cy: number])[] = [
  [5, 3],
  [11, 3],
  [11, 8],
  [11, 13],
  [5, 13],
  [5, 8],
];
/** The radius of each busy-mark dot on the 16x16 viewBox. */
export const busyDotRadius = 1.6;
const SVG = 'http://www.w3.org/2000/svg';

/** The busy mark's SVG (its dots in currentColor), as `BusyMark` renders it. */
function busySvg(doc: Document): SVGSVGElement {
  const svg = doc.createElementNS(SVG, 'svg');
  svg.setAttribute('viewBox', '0 0 16 16');
  for (const [cx, cy] of busyDots) {
    const dot = doc.createElementNS(SVG, 'circle');
    dot.setAttribute('class', browserUiClasses.busyDot);
    dot.setAttribute('cx', String(cx));
    dot.setAttribute('cy', String(cy));
    dot.setAttribute('r', String(busyDotRadius));
    svg.append(dot);
  }
  return svg;
}

/** The busy mark alone, for a framework-free wait such as a route that is still loading. */
export function createBusyMark(doc: Document = document): HTMLElement {
  const busy = doc.createElement('span');
  busy.className = browserUiClasses.busyMark;
  busy.append(busySvg(doc));
  return busy;
}

export function createActionMark(doc: Document = document): HTMLElement {
  const mark = doc.createElement('span');
  mark.className = browserUiClasses.actionMark;
  mark.setAttribute('aria-hidden', 'true');
  mark.dataset.busy = 'false';
  mark.append(createBusyMark(doc));
  return mark;
}

/** A line icon from the shared icon nodes, for a framework-free page: it takes the text colour. */
export function createIcon(name: ShellIcon, doc: Document = document): SVGSVGElement {
  const svg = doc.createElementNS(SVG, 'svg');
  svg.setAttribute('class', browserUiClasses.glyph);
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('aria-hidden', 'true');
  for (const [tag, attributes] of shellIconNodes[name]) {
    const node = doc.createElementNS(SVG, tag);
    for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
    svg.append(node);
  }
  return svg;
}
