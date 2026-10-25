/** Host drivers and agent hooks: read-only records and the terminal commands that read them. */
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { commandCopy } from './command-copy.js';
import { notAvailable, notInBrowser } from './not-available.js';
import type { DriverRow, HookRow, TmtSettings } from './settings-source.js';
import { DRIVER_COPY, HOOK_COPY, displayName } from './tmt-settings-copy.js';
import {
  element,
  settingsCard,
  settingsHead,
  settingsNotice,
  settingsReadOnly,
  settingsRecord,
  settingsRecords,
  settingsState,
} from './tmt-settings-parts.js';

function driverRecord(row: DriverRow): HTMLElement {
  const approved = row.state === 'ok';
  const copy = DRIVER_COPY[row.name];
  return settingsRecord({
    icon: 'box',
    name: displayName(row.name),
    ...(copy ? { description: copy } : {}),
    facts: [
      // A driver that no longer runs as approved names its reason, in the waiting tone.
      settingsState(approved ? 'Approved' : row.state, approved ? 'working' : 'waiting'),
      element('span', 'tmt-settings-fact tmt-settings-mono', row.version),
    ],
  });
}

function hookRecord(row: HookRow): HTMLElement {
  const copy = HOOK_COPY[row.provider];
  return settingsRecord({
    icon: 'terminal',
    name: displayName(row.provider),
    ...(copy ? { description: copy } : {}),
    facts: [
      element('span', 'tmt-settings-fact', row.usage ? 'Collection on' : 'Collection off'),
      settingsState('Set up', 'working'),
    ],
  });
}

/** The two roles, in words about the product; no value of this machine. */
function rolesCard(): HTMLElement {
  const roles = element('aside', `${ui.card} tmt-settings-card`);
  roles.append(
    element('h2', ui.cardTitle, 'Different connections. Clear roles.'),
    element(
      'p',
      'tmt-settings-help',
      'Host drivers connect TMT to terminal host windows. Agent hooks report provider events and can collect usage for Digest.',
    ),
    element('hr', 'tmt-settings-rule'),
    element(
      'p',
      'tmt-settings-help',
      'The browser observes these records. Approval and installation stay with their existing terminal commands.',
    ),
  );
  return roles;
}

export function driversView(settings: TmtSettings): HTMLElement[] {
  const unavailable = settings.drivers.unavailable ?? settings.hooks.unavailable;
  const drivers = settings.drivers.rows;
  const hooks = settings.hooks.rows;
  const none = !drivers.length && !hooks.length;
  const head = settingsHead(
    'Drivers & hooks',
    'The connections that bring your terminal team together.',
  );
  const commands = element('div', 'tmt-settings-commands');
  commands.append(commandCopy('tmt driver ls'), commandCopy('tmt setup --status'));
  if (unavailable || none) {
    const notice = settingsNotice(
      unavailable
        ? 'Driver and hook records are not available to this browser yet.'
        : 'No drivers or agent hooks are set up.',
      'Read current drivers and hook setup in your terminal.',
      unavailable ? 'waiting' : 'neutral',
    );
    if (unavailable && unavailable !== notInBrowser('TMT settings', 'tmt config show')) {
      const reason = element('p');
      notAvailable(reason, unavailable);
      notice.append(reason);
    }
    return [
      head,
      notice,
      commands,
      rolesCard(),
      element('p', 'tmt-settings-foot', 'Read only · Current records require a machine read'),
    ];
  }
  const notice = settingsNotice(
    settings.sample ? 'Sample data' : 'Read-only driver and hook records',
    settings.sample
      ? 'These sample records are not this machine. Read the current ones in your terminal.'
      : 'Approval and installation stay with their terminal commands.',
    'neutral',
  );
  const stack = element('div', 'tmt-settings-stack');
  if (drivers.length) {
    const card = settingsCard('Host drivers', 'tmt-settings-drivers-title');
    card.append(settingsReadOnly());
    const list = settingsRecords('Host drivers');
    list.append(...drivers.map(driverRecord));
    card.append(list);
    stack.append(card);
  }
  if (hooks.length) {
    const card = settingsCard('Agent hooks', 'tmt-settings-hooks-title');
    const list = settingsRecords('Agent hooks');
    list.append(...hooks.map(hookRecord));
    card.append(list);
    stack.append(card);
  }
  const grid = element('div', 'tmt-settings-grid');
  grid.append(stack, rolesCard());
  return [
    head,
    notice,
    commands,
    grid,
    element(
      'p',
      'tmt-settings-foot',
      settings.sample
        ? 'Sample records · Read only'
        : 'Read only · Current records require a machine read',
    ),
  ];
}
