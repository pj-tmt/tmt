export const browserUiClasses = Object.freeze({
  chips: 'tmt-ui-chips',
  chip: 'tmt-ui-chip',
  chipLabel: 'tmt-ui-chip-label',
  chipsAdd: 'tmt-ui-chips-add',
  chipsStatus: 'tmt-ui-chips-status',
  chipsSurface: 'tmt-ui-chips-surface',
  chipsSearch: 'tmt-ui-chips-search',
  chipsOption: 'tmt-ui-chips-option',

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
  fieldDescription: 'tmt-ui-field-description',
  fieldError: 'tmt-ui-field-error',
  action: 'tmt-ui-action',
  actionLabel: 'tmt-ui-action-label',
  actionMark: 'tmt-ui-action-mark',
  busyMark: 'tmt-ui-busy-mark',
  busyFrames: 'tmt-ui-busy-frames',
  checkbox: 'tmt-ui-checkbox',
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
  listUpdated: 'tmt-ui-list-updated',
  listState: 'tmt-ui-list-state',
  listActions: 'tmt-ui-list-actions',
  tabs: 'tmt-ui-tabs',
  tabsHeader: 'tmt-ui-tabs-header',
  tabList: 'tmt-ui-tab-list',
  tab: 'tmt-ui-tab',
  tabMark: 'tmt-ui-tab-mark',
  tabLabel: 'tmt-ui-tab-label',
  tabsStatus: 'tmt-ui-tabs-status',
  tabPanel: 'tmt-ui-tab-panel',
} as const);

export type BrowserNoticeTone = 'waiting' | 'blocked' | 'working' | 'review' | 'muted';
export interface BrowserTab {
  id: string;
  tabId: string;
  panelId: string;
  label: string;
  state?: { mark: string; word: string; tone: BrowserNoticeTone };
}
export type BrowserTabSelectReason = 'keyboard' | 'activation';
export type BrowserActionVariant = 'text' | 'primary' | 'destructive';
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
 * The reserved busy-mark slot of a static Action: a leading span holding the shared LoaderCircle
 * arc, hidden while `data-busy` is `false`. Set `data-busy` to `true` from `createBusyMarkGate`;
 * the shared spinner rule turns it. Non-React pages use this instead of copying the markup.
 */
export function createActionMark(doc: Document = document): HTMLElement {
  const mark = doc.createElement('span');
  mark.className = browserUiClasses.actionMark;
  mark.setAttribute('aria-hidden', 'true');
  mark.dataset.busy = 'false';
  const svg = doc.createElementNS('http://www.w3.org/2000/svg', 'svg');
  for (const [name, value] of Object.entries({
    width: '1em',
    height: '1em',
    viewBox: '0 0 24 24',
    fill: 'none',
    stroke: 'currentColor',
    'stroke-width': '2',
    'stroke-linecap': 'round',
    'stroke-linejoin': 'round',
  }))
    svg.setAttribute(name, value);
  const arc = doc.createElementNS('http://www.w3.org/2000/svg', 'path');
  arc.setAttribute('d', 'M21 12a9 9 0 1 1-6.219-8.56');
  svg.append(arc);
  mark.append(svg);
  return mark;
}
