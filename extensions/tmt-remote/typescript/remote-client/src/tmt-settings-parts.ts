/** Stateless DOM pieces shared by TMT Settings sections. */
import { browserUiClasses } from '@tmt/browser-ui/static';
import { createIcon } from '@tmt/browser-ui/glyph';

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

export function settingsHead(title: string, subtitle: string): HTMLElement {
  const head = element('div', browserUiClasses.pageHead);
  head.append(
    element('h2', browserUiClasses.pageTitle, title),
    element('p', browserUiClasses.pageSubtitle, subtitle),
  );
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

export function settingsNotice(title: string, description: string): HTMLElement {
  const warning = element('div', 'tmt-settings-warning');
  warning.setAttribute('role', 'note');
  const lead = element('strong');
  lead.append(createIcon('triangle-alert'), element('span', undefined, title));
  warning.append(lead, element('p', undefined, description));
  return warning;
}
