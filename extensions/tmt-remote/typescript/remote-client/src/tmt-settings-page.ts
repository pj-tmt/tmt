/** TMT Settings follows the shell's section menu. General owns browser appearance and Core
 * guidance; Identities has a read-only table and local detail; Modules and Drivers & hooks list
 * read-only records. SettingsSource owns the records; only the shared browser theme choice is
 * persisted. */
import { driversView } from './tmt-settings-drivers.js';
import { identitiesView } from './tmt-settings-identities.js';
import { modulesView } from './tmt-settings-modules.js';
import {
  element,
  settingsCard,
  settingsHead,
  settingsNotice,
  settingsReadOnly,
} from './tmt-settings-parts.js';
import { commandCopy } from './command-copy.js';
import { pageLoading } from './page-loading.js';
import { notAvailable, notInBrowser } from './not-available.js';
import { createIcon } from '@tmt/browser-ui/glyph';
import { browserUiClasses } from '@tmt/browser-ui/static';
import type { ShellAnswer } from '@tmt/browser-ui/shell';
import { THEMES, setStoredTheme, storedTheme, type ThemeChoice } from '@tmt/browser-ui/theme';
import type { RemoteShell } from './remote-shell.js';
import { settingsSource, type SettingsSource, type TmtSettings } from './settings-source.js';

export const SECTIONS = [
  { id: 'general', label: 'General' },
  { id: 'identities', label: 'Identities' },
  { id: 'modules', label: 'Modules' },
  { id: 'drivers', label: 'Drivers & hooks' },
] as const;
export type Section = (typeof SECTIONS)[number]['id'];

const THEME_LABELS: Record<ThemeChoice, string> = {
  system: 'System',
  light: 'Light',
  dark: 'Dark',
};

/** One list row: a title, a detail and a state word. */
interface SettingsRow {
  title: string;
  detail: string;
  state: string;
}

/** A module's own settings page, from the composed shell's description of it. */
export interface ModuleLink {
  label: string;
  href?: string;
}

/** Each described module's rail label and first settings page, by extension name. */
export function moduleLinks(answer: ShellAnswer | undefined): Map<string, ModuleLink> {
  const links = new Map<string, ModuleLink>();
  for (const module of answer?.modules ?? [])
    links.set(module.extension, { label: module.rail.label, href: module.settings[0]?.path });
  return links;
}

/** General's rows: each setting's name, description and value. */
const generalRows = (settings: TmtSettings): SettingsRow[] =>
  settings.general.rows.map((row) => ({
    title: row.name,
    detail: row.description,
    state: row.value,
  }));

function row(item: SettingsRow): HTMLLIElement {
  const node = document.createElement('li');
  node.className = 'tmt-ui-list-row';
  node.dataset.leading = '';
  const sign = document.createElement('div');
  sign.className = 'tmt-ui-list-leading';
  sign.setAttribute('aria-hidden', 'true');
  // The label and its description stack on the left; the state, the value, sits on the right.
  const name = document.createElement('div');
  name.className = 'tmt-ui-list-title';
  const label = document.createElement('span');
  label.className = 'tmt-ui-list-label';
  label.textContent = item.title;
  name.append(label);
  if (item.detail) {
    const detail = document.createElement('span');
    detail.className = 'tmt-ui-list-description';
    detail.textContent = item.detail;
    name.append(detail);
  }
  const info = document.createElement('div');
  info.className = 'tmt-ui-list-meta';
  const state = document.createElement('div');
  state.className = 'tmt-ui-list-state';
  state.textContent = item.state;
  info.append(state);
  node.append(sign, name, info);
  return node;
}

/** The theme choice: one tab per theme; picking one stores it for this browser and applies it. */
function themeChoice(): HTMLElement {
  const group = document.createElement('div');
  group.className = 'tmt-settings-themes';
  group.setAttribute('role', 'tablist');
  group.setAttribute('aria-label', 'Theme');
  let current = storedTheme();
  const buttons = THEMES.map((theme) => {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'tmt-settings-theme';
    button.setAttribute('role', 'tab');
    button.dataset.choice = theme;
    const label = element('span', 'tmt-settings-theme-label');
    const radio = element('span', 'tmt-settings-theme-radio');
    radio.setAttribute('aria-hidden', 'true');
    label.append(radio, element('span', undefined, THEME_LABELS[theme]));
    button.append(themePreview(theme), label);
    return button;
  });
  const show = (theme: ThemeChoice) => {
    current = theme;
    for (const button of buttons) {
      const selected = button.dataset.choice === theme;
      button.setAttribute('aria-selected', String(selected));
      button.tabIndex = selected ? 0 : -1;
    }
  };
  const choose = (theme: ThemeChoice) => {
    setStoredTheme(theme);
    show(theme);
  };
  for (const button of buttons) {
    button.addEventListener('click', () => choose(button.dataset.choice as ThemeChoice));
    button.addEventListener('keydown', (event) => {
      const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
      if (!step) return;
      event.preventDefault();
      const next = THEMES[(THEMES.indexOf(current) + step + THEMES.length) % THEMES.length]!;
      choose(next);
      buttons[THEMES.indexOf(next)]!.focus();
    });
  }
  show(current);
  group.append(...buttons);
  return group;
}

function themePreview(theme: ThemeChoice): HTMLElement {
  const preview = element('span', 'tmt-settings-theme-preview');
  preview.setAttribute('aria-hidden', 'true');
  for (const choice of theme === 'system' ? ['dark', 'light'] : [theme]) {
    const half = element('span', 'tmt-settings-theme-preview-half');
    half.setAttribute('data-theme', choice);
    const side = element('span', 'tmt-settings-theme-preview-side');
    const main = element('span', 'tmt-settings-theme-preview-main');
    for (let index = 0; index < 4; index += 1) {
      side.append(element('i'));
      main.append(element('i'));
    }
    half.append(side, main);
    preview.append(half);
  }
  return preview;
}

function generalView(settings: TmtSettings): HTMLElement[] {
  const head = settingsHead(
    'Make TMT feel like yours.',
    'A familiar space, with just the right amount of you.',
  );
  const appearance = settingsCard('Appearance', 'tmt-settings-appearance-title');
  appearance.append(
    element('p', 'tmt-settings-intro', 'Choose how TMT looks in this browser.'),
    themeChoice(),
    element('p', 'tmt-settings-help', 'Saved in this browser only. System follows this device.'),
  );
  const core = settingsCard('Core configuration', 'tmt-settings-core-title');
  core.append(settingsReadOnly());
  const rows = generalRows(settings);
  if (rows.length) {
    const list = element('ul', 'tmt-ui-list');
    list.append(...rows.map(row));
    core.append(list);
  }
  if (!rows.length || settings.general.unavailable) {
    const warning = settingsNotice(
      settings.general.unavailable
        ? 'This browser cannot read Core settings yet.'
        : 'No settings to show.',
      'View machine configuration in your terminal.',
    );
    const reason = settings.general.unavailable;
    if (reason && reason !== notInBrowser('TMT settings', 'tmt config show')) {
      const detail = element('p');
      notAvailable(detail, reason);
      warning.append(detail);
    }
    warning.append(commandCopy('tmt config show'));
    core.append(warning);
  } else core.append(commandCopy('tmt config show'));
  const primary = element('div', 'tmt-settings-stack');
  primary.append(appearance, core);
  const machine = settingsCard('Your machine.', 'tmt-settings-machine-title');
  machine.firstElementChild!.append(
    element('br'),
    element('span', undefined, 'Your configuration.'),
  );
  const tile = element('span', browserUiClasses.glyphTile);
  tile.append(createIcon('terminal'));
  machine.prepend(tile);
  const machineText = element('p', 'tmt-settings-help');
  machineText.append(
    element('span', undefined, 'Core settings live on your machine. Use '),
    element('code', undefined, 'tmt config show'),
    element('span', undefined, ' to read the current values.'),
  );
  const link = element('a', 'tmt-settings-link', 'Browse modules →');
  link.href = `${location.pathname}#modules`;
  machine.append(
    machineText,
    element('hr', 'tmt-settings-rule'),
    element('h3', undefined, 'Looking for a module?'),
    element('p', 'tmt-settings-help', 'Find each module’s own settings, together in one place.'),
    link,
  );
  const browser = settingsCard('Browser appearance', 'tmt-settings-browser-title');
  browser.append(
    element(
      'p',
      'tmt-settings-help',
      'Appearance stays local to this browser. Machine configuration is managed with the existing terminal commands.',
    ),
  );
  const side = element('aside', 'tmt-settings-stack');
  side.append(machine, browser);
  const grid = element('div', 'tmt-settings-grid');
  grid.append(primary, side);
  return [
    head,
    grid,
    element(
      'p',
      'tmt-settings-foot',
      'Browser appearance · Machine configuration stays in the terminal',
    ),
  ];
}

/** The section's content. */
export function settingsSection(
  section: Section,
  settings: TmtSettings,
  links?: ReadonlyMap<string, ModuleLink>,
): HTMLElement[] {
  if (section === 'general') return generalView(settings);
  if (section === 'identities') return identitiesView(settings);
  if (section === 'modules') return modulesView(settings, links ?? new Map());
  return driversView(settings);
}

/** The page in its route; everything it starts ends on the route's `shell.signal`. */
export function tmtSettingsPage(shell: RemoteShell, source: SettingsSource = settingsSource): void {
  const main = document.getElementById('tmt-settings-section');
  const note = document.getElementById('tmt-settings-note');
  if (!main || !note) return;
  const title = document.querySelector<HTMLElement>('.tmt-ui-title');
  const href = (id: Section) => `${location.pathname}#${id}`;
  const current = (): Section =>
    SECTIONS.find((section) => `#${section.id}` === location.hash)?.id ?? 'general';
  let settings: TmtSettings | undefined;
  let links: Map<string, ModuleLink> = new Map();
  const live = () => !shell.signal.aborted;
  const paint = () => {
    if (!live()) return;
    const section = current();
    if (settings) {
      note.replaceChildren();
      // Only General keeps the page-level sample note; the other sections label their own records.
      note.hidden = !settings.sample || section !== 'general';
      if (!note.hidden)
        notAvailable(
          note,
          'Sample data: Core has no web read of this machine yet. `tmt ls`, `tmt extension ls` and `tmt driver ls` show the real values.',
        );
    }
    const page = document.getElementById('tmt-settings-page');
    if (page) {
      page.dataset.section = section;
      page.className = 'tmt-page tmt-settings tmt-ui-page';
    }
    shell.setMenu({
      title: 'TMT settings',
      groups: [
        {
          items: SECTIONS.map((item) => ({
            id: item.id,
            label: item.label,
            href: href(item.id),
            current: item.id === section,
          })),
        },
      ],
    });
    if (title) title.textContent = SECTIONS.find((item) => item.id === section)!.label;
    main.replaceChildren(
      ...(settings ? settingsSection(section, settings, links) : [pageLoading()]),
    );
  };
  window.addEventListener('hashchange', paint, { signal: shell.signal });
  shell.described((answer) => {
    links = moduleLinks(answer);
    paint();
  });
  paint();
  void source.read().then(
    (next) => {
      if (!live()) return;
      settings = next;
      paint();
    },
    () => {
      if (!live()) return;
      main.replaceChildren();
      note.textContent = 'Settings could not be read.';
      note.hidden = false;
    },
  );
}
