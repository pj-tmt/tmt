/** Stateless DOM pieces shared by TMT Settings sections. */
import { browserUiClasses } from '@tmt/browser-ui/static';
import { createIcon } from '@tmt/browser-ui/glyph';
import type { SettingsIcon } from './tmt-settings-copy.js';

export function element<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

export function settingsHead(subtitle: string): HTMLElement {
  const head = element('div', browserUiClasses.pageHead);
  if (subtitle) head.append(element('p', browserUiClasses.pageSubtitle, subtitle));
  return head;
}

export function settingsCard(title: string, id: string): HTMLElement {
  const card = element('section', `${browserUiClasses.card} tmt-settings-card`);
  card.setAttribute('aria-labelledby', id);
  const heading = element('h2', browserUiClasses.cardTitle, title);
  heading.id = id;
  card.append(heading);
  return card;
}

export function settingsReadOnly(): HTMLElement {
  const badge = element('span', `${browserUiClasses.chip} tmt-settings-read-only`);
  badge.append(createIcon('lock'), element('span', browserUiClasses.chipLabel, 'Read only'));
  return badge;
}

export function settingsNotice(
  title: string,
  description: string,
  tone: 'waiting' | 'neutral' = 'waiting',
): HTMLElement {
  const warning = element('div', 'tmt-settings-warning');
  warning.setAttribute('role', 'note');
  warning.dataset.tone = tone;
  const lead = element('strong');
  lead.append(
    createIcon(tone === 'waiting' ? 'triangle-alert' : 'terminal'),
    element('span', undefined, title),
  );
  warning.append(lead, element('p', undefined, description));
  return warning;
}

/** One state chip: a mark and a word, never colour alone. */
export function settingsState(label: string, tone: 'working' | 'waiting' | 'muted'): HTMLElement {
  const chip = element('span', browserUiClasses.chip);
  chip.dataset.tone = tone;
  const mark = element('span', browserUiClasses.stateMark);
  mark.dataset.shape = tone === 'muted' ? 'ring' : 'dot';
  mark.setAttribute('aria-hidden', 'true');
  chip.append(mark, element('span', browserUiClasses.chipLabel, label));
  return chip;
}

/** The list of one record kind. */
export function settingsRecords(label: string): HTMLUListElement {
  const list = element('ul', 'tmt-settings-records');
  list.setAttribute('aria-label', label);
  return list;
}

/** One record: a glyph tile, a name and a line, then its facts and one optional action. At narrow
 * widths the action stays at the end of the name row and the facts sit on the line below. */
export function settingsRecord(record: {
  icon: SettingsIcon;
  name: string;
  description?: string;
  facts?: readonly (string | HTMLElement)[];
  action?: HTMLElement;
}): HTMLLIElement {
  const item = element('li', 'tmt-settings-record');
  const tile = element('span', browserUiClasses.glyphTile);
  tile.append(createIcon(record.icon));
  const body = element('div', 'tmt-settings-record-body');
  body.append(element('strong', undefined, record.name));
  if (record.description) body.append(element('span', 'tmt-settings-help', record.description));
  const facts = element('div', 'tmt-settings-record-facts');
  for (const fact of record.facts ?? [])
    facts.append(typeof fact === 'string' ? element('span', 'tmt-settings-fact', fact) : fact);
  const meta = element('div', 'tmt-settings-record-meta');
  meta.append(facts);
  if (record.action) meta.append(record.action);
  item.append(tile, body, meta);
  return item;
}

/** A link in the shape of a small secondary action. */
export function settingsAction(label: string, href: string): HTMLAnchorElement {
  const link = element('a', `${browserUiClasses.action} tmt-settings-action`, label);
  link.href = href;
  return link;
}
