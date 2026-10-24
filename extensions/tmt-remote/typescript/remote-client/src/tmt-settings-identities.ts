/** Identity inventory presentation. Selection belongs only to this rendered view. */
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { createIcon } from '@tmt/browser-ui/glyph';
import { commandCopy } from './command-copy.js';
import { notAvailable, notInBrowser } from './not-available.js';
import type { IdentityRow, TmtSettings } from './settings-source.js';
import {
  element,
  settingsCard,
  settingsHead,
  settingsNotice,
  settingsReadOnly,
} from './tmt-settings-parts.js';

/** Keep the existing live-first, then name order without changing the source records. */
export function orderedIdentities(rows: readonly IdentityRow[]): IdentityRow[] {
  return [...rows].sort(
    (a, b) => Number(b.online) - Number(a.online) || a.name.localeCompare(b.name),
  );
}

function providerLabel(provider?: string): string {
  if (!provider) return 'Not reported';
  return provider === 'codex' ? 'Codex' : provider === 'claude' ? 'Claude' : provider;
}

function presence(identity: IdentityRow): HTMLElement {
  const chip = element('span', ui.chip);
  chip.dataset.tone = identity.online ? 'working' : 'muted';
  const mark = element('span', ui.stateMark);
  mark.dataset.shape = identity.online ? 'dot' : 'ring';
  mark.setAttribute('aria-hidden', 'true');
  chip.append(mark, element('span', ui.chipLabel, identity.online ? 'Active' : 'Offline'));
  return chip;
}

export function identitiesView(settings: TmtSettings): HTMLElement[] {
  const rows = orderedIdentities(settings.identities.rows);
  const unavailable = settings.identities.unavailable;
  const notice = settingsNotice(
    unavailable
      ? 'Web data connection pending'
      : settings.sample
        ? 'Sample data'
        : 'Read-only identity records',
    unavailable
      ? 'Core identities are not available in this browser yet. Run tmt ls to see this machine’s identities.'
      : settings.sample
        ? 'These sample records are not this machine. Use tmt ls in your terminal to read identities.'
        : 'Manage identities in your terminal. Use tmt ls to inspect the current machine records.',
  );
  notice.className += ' tmt-settings-identity-notice';
  const specificReason =
    unavailable && unavailable !== notInBrowser('TMT settings', 'tmt config show');
  if (specificReason) {
    const reason = element('p');
    notAvailable(reason, unavailable);
    notice.append(reason);
  }
  notice.append(commandCopy('tmt ls'));
  const inventory = settingsCard(
    settings.sample ? 'Sample identities' : 'Identities',
    'tmt-settings-identities-title',
  );
  inventory.append(settingsReadOnly());
  if (settings.sample)
    inventory.append(element('p', 'tmt-settings-help', 'Sample records · Not this machine'));
  const detail = element('aside', `${ui.card} tmt-settings-card tmt-settings-identity-detail`);
  detail.id = 'tmt-settings-identity-detail';
  detail.setAttribute('aria-live', 'polite');
  const explanation =
    'Identity lifetime and current presence are separate. A saved identity remains when its agent goes offline.';
  if (!rows.length) {
    const empty = element('div', 'tmt-settings-identity-empty');
    empty.append(
      element(
        'strong',
        undefined,
        unavailable ? 'Identity inventory unavailable' : 'No identities yet.',
      ),
    );
    const reason = element('p', 'tmt-settings-help');
    reason.textContent = unavailable
      ? 'No machine records are inferred. Saved identities and current presence are separate facts.'
      : 'The source supplied no identity records.';
    empty.append(reason);
    inventory.append(empty);
    const tile = element('span', ui.glyphTile);
    tile.append(createIcon('terminal'));
    detail.append(
      tile,
      element('h2', ui.cardTitle, 'Identity lifetime & presence'),
      element('p', 'tmt-settings-help', explanation),
    );
  } else {
    const table = element('table', `${ui.table} tmt-settings-identity-table`);
    table.dataset.roomy = '';
    table.setAttribute('aria-label', 'Identities');
    const head = element('thead');
    const header = element('tr');
    for (const name of ['Name', 'Lifetime', 'Provider', 'Presence']) {
      const cell = element('th', undefined, name);
      cell.scope = 'col';
      header.append(cell);
    }
    head.append(header);
    const body = element('tbody');
    const entries: { identity: IdentityRow; row: HTMLElement; button: HTMLButtonElement }[] = [];
    const select = (identity: IdentityRow) => {
      for (const entry of entries) {
        const current = entry.identity === identity;
        entry.button.setAttribute('aria-current', String(current));
        if (current) entry.row.dataset.selected = '';
        else delete entry.row.dataset.selected;
      }
      const title = element('h2', ui.cardTitle, identity.name);
      title.id = 'tmt-settings-identity-detail-title';
      detail.setAttribute('aria-labelledby', title.id);
      const list = element('dl', 'tmt-settings-identity-facts');
      for (const [label, value] of [
        ['Lifetime', identity.lifetime === 'saved' ? 'Saved' : 'Temporary'],
        ['Provider', providerLabel(identity.provider)],
        ['Presence', presence(identity)],
      ] as const) {
        const valueNode = element('dd');
        valueNode.append(value);
        list.append(element('dt', undefined, label), valueNode);
      }
      detail.replaceChildren(title, list, element('p', 'tmt-settings-help', explanation));
    };
    for (const identity of rows) {
      const tr = element('tr');
      const name = element('td');
      const button = element(
        'button',
        `${ui.tableSelect} tmt-settings-identity-name`,
        identity.name,
      );
      button.type = 'button';
      button.setAttribute('aria-controls', detail.id);
      button.addEventListener('click', () => select(identity));
      entries.push({ identity, row: tr, button });
      name.append(button);
      const state = element('td');
      state.append(presence(identity));
      tr.append(
        name,
        element('td', undefined, identity.lifetime === 'saved' ? 'Saved' : 'Temporary'),
        element('td', undefined, providerLabel(identity.provider)),
        state,
      );
      body.append(tr);
    }
    table.append(head, body);
    inventory.append(table);
    select(rows[0]!);
  }
  const grid = element('div', 'tmt-settings-grid');
  grid.append(inventory, detail);
  return [
    settingsHead('Identity management', 'The names behind your terminal team.'),
    notice,
    grid,
    element(
      'p',
      'tmt-settings-foot',
      settings.sample
        ? 'Sample identities · Not this machine'
        : 'Read only · Current records require a machine read',
    ),
  ];
}
