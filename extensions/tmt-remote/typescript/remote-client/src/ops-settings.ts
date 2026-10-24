/** The Ops settings page. The board's settings have no browser read, so the page says where they
 * are (the terminal workspace, one command to copy) and points at the Ops views that are here. */
import { browserUiClasses } from '@tmt/browser-ui/static';
import { createIcon } from '@tmt/browser-ui/glyph';
import { el, pageFoot, pageHead, terminalCard } from './ops-parts.js';

export function settingsView(href: (section: 'agents' | 'squads') => string): HTMLElement {
  const page = el('div', 'ops-settings-page');
  const keep = el('section', `${browserUiClasses.card} ops-settings-keep`);
  keep.setAttribute('aria-labelledby', 'ops-keep-title');
  const text = el('div');
  const title = el('h3', browserUiClasses.cardTitle, 'Keep a clear view of your team');
  title.id = 'ops-keep-title';
  text.append(
    title,
    el('p', 'ops-settings-note', 'Agents, squads and timers are available as read-only web views.'),
  );
  const links = el('div', 'ops-settings-links');
  const link = (section: 'agents' | 'squads', label: string, next?: boolean) => {
    const anchor = el('a', browserUiClasses.action);
    anchor.href = href(section);
    anchor.append(el('span', browserUiClasses.actionLabel, label));
    if (next) anchor.append(createIcon('chevron-right'));
    return anchor;
  };
  links.append(link('agents', 'View agents', true), link('squads', 'View squads'));
  keep.append(text, links);
  page.append(
    pageHead('Ops settings', 'Manage your workspace from the terminal.'),
    terminalCard({
      id: 'ops-settings-title',
      title: 'The right controls, in your terminal.',
      text: 'Ops settings are not available in the browser yet. Open the terminal workspace to view and manage them.',
      hero: true,
    }),
    keep,
    pageFoot('CLI-managed settings'),
  );
  return page;
}
