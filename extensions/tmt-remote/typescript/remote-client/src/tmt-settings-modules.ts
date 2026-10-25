/** Installed modules: the module list with each module's own settings, from the shell's description. */
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { createIcon } from '@tmt/browser-ui/glyph';
import { commandCopy } from './command-copy.js';
import { notAvailable, notInBrowser } from './not-available.js';
import type { ModuleRow, TmtSettings } from './settings-source.js';
import { MODULE_COPY, displayName } from './tmt-settings-copy.js';
import type { ModuleLink } from './tmt-settings-page.js';
import {
  element,
  settingsAction,
  settingsCard,
  settingsHead,
  settingsNotice,
  settingsReadOnly,
  settingsRecord,
  settingsRecords,
  settingsState,
} from './tmt-settings-parts.js';

function moduleRecord(row: ModuleRow, link: ModuleLink | undefined): HTMLElement {
  const known = MODULE_COPY[row.name];
  const name = link?.label ?? displayName(row.name);
  const facts: (string | HTMLElement)[] = [
    settingsState(
      row.installed ? 'Installed' : 'Not installed',
      row.installed ? 'working' : 'muted',
    ),
  ];
  // A version that already names its channel (`0.1.0-alpha.1`) is not repeated beside it.
  const detail =
    row.channel && row.version?.includes(row.channel)
      ? row.version
      : [row.channel, row.version].filter(Boolean).join(' · ');
  if (row.installed && detail)
    facts.push(element('span', 'tmt-settings-fact tmt-settings-mono', detail));
  return settingsRecord({
    icon: known?.icon ?? 'box',
    name,
    ...(known ? { description: known.description } : {}),
    facts,
    ...(row.installed && link?.href ? { action: settingsAction('Settings', link.href) } : {}),
  });
}

export function modulesView(
  settings: TmtSettings,
  links: ReadonlyMap<string, ModuleLink>,
): HTMLElement[] {
  const rows = settings.modules.rows;
  const unavailable = settings.modules.unavailable;
  const head = settingsHead(
    'Modules, in one place.',
    'A few focused tools. One connected workspace.',
  );
  const out: HTMLElement[] = [head];
  if (unavailable || !rows.length) {
    const notice = settingsNotice(
      unavailable
        ? 'Module inventory is not available to this browser yet.'
        : 'No modules to show.',
      'Read installed modules in your terminal.',
    );
    if (unavailable && unavailable !== notInBrowser('TMT settings', 'tmt config show')) {
      const reason = element('p');
      notAvailable(reason, unavailable);
      notice.append(reason);
    }
    notice.append(commandCopy('tmt extension ls'));
    out.push(
      notice,
      element('p', 'tmt-settings-foot', 'Read only · Current records require a machine read'),
    );
    return out;
  }
  const notice = settingsNotice(
    settings.sample ? 'Sample data' : 'Read-only module records',
    settings.sample
      ? 'These sample records are not this machine. Use tmt extension ls in your terminal to read modules.'
      : 'Manage modules in your terminal. Use tmt extension ls to inspect the current records.',
    'neutral',
  );
  notice.append(commandCopy('tmt extension ls'));
  const card = settingsCard(
    settings.sample ? 'Sample installed modules' : 'Installed modules',
    'tmt-settings-modules-title',
  );
  card.append(settingsReadOnly());
  const list = settingsRecords('Modules');
  list.append(...rows.map((row) => moduleRecord(row, links.get(row.name))));
  card.append(list);
  const grid = element('div', 'tmt-settings-grid');
  grid.append(card);
  const remote = links.get('remote');
  if (rows.some((row) => row.name === 'remote' && row.installed) && remote?.href) {
    const aside = element('aside', `${ui.card} tmt-settings-card`);
    const tile = element('span', ui.glyphTile);
    tile.append(createIcon('monitor-smartphone'));
    const link = element('a', 'tmt-settings-link', 'Open Remote settings →');
    link.href = remote.href;
    aside.append(
      tile,
      element('h2', ui.cardTitle, 'Remote'),
      element(
        'p',
        'tmt-settings-help',
        'Pair browsers with this machine and manage their access. Each device keeps its own pairing and permissions.',
      ),
      element('hr', 'tmt-settings-rule'),
      link,
      element('hr', 'tmt-settings-rule'),
      element('p', 'tmt-settings-help', 'Source · Local CLI'),
      commandCopy('tmt extension ls'),
    );
    grid.append(aside);
  }
  out.push(
    notice,
    grid,
    element(
      'p',
      'tmt-settings-foot',
      settings.sample
        ? 'Sample inventory · No install or update actions'
        : 'Read only · No install or update actions',
    ),
  );
  return out;
}
