/**
 * TMT's own settings (`/settings`), one section at a time from the module menu: General (theme,
 * then read-only values), Identities, Modules and Drivers & hooks. Everything but the theme is
 * read-only; rows are the shared list rows and every state is a mark and a word.
 */
import type { ShellAnswer } from '../../../../../design/browser-ui/src/shell.js';
import {
  THEMES,
  setStoredTheme,
  storedTheme,
  type ThemeChoice,
} from '../../../../../design/browser-ui/src/theme.js';
import { remoteShell } from './remote-shell.js';
import {
  settingsSource,
  type Part,
  type SettingsSource,
  type TmtSettings,
} from './settings-source.js';

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

/** One shared list row: a mark, a title (a link when `href` is set), a detail and a state word. */
export interface SettingsRow {
  mark: string;
  waiting?: boolean;
  title: string;
  href?: string;
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

const capitalize = (name: string) => name.charAt(0).toUpperCase() + name.slice(1);

/** The section's rows; identities show live agents first, then by name. */
export function settingsRows(
  section: Section,
  settings: TmtSettings,
  links: ReadonlyMap<string, ModuleLink> = new Map(),
): SettingsRow[] {
  if (section === 'general')
    return settings.general.rows.map((row) => ({
      mark: '',
      title: row.name,
      detail: row.description,
      state: row.value,
    }));
  if (section === 'identities')
    return [...settings.identities.rows]
      .sort((a, b) => Number(b.online) - Number(a.online) || a.name.localeCompare(b.name))
      .map((row) => ({
        mark: row.online ? '●' : '○',
        title: row.name,
        detail: row.provider ? `${row.lifetime} · ${row.provider}` : row.lifetime,
        state: row.online ? 'active' : 'offline',
      }));
  if (section === 'modules')
    return settings.modules.rows.map((row) => {
      const link = links.get(row.name);
      return {
        mark: row.installed ? '●' : '○',
        title: link?.label ?? capitalize(row.name),
        ...(row.installed && link?.href ? { href: link.href } : {}),
        detail: row.installed
          ? [row.version, row.channel].filter(Boolean).join(' · ')
          : 'Not installed',
        state: row.installed ? 'installed' : '',
      };
    });
  return [
    ...settings.drivers.rows.map((row) => ({
      mark: row.state === 'ok' ? '●' : '◆',
      waiting: row.state !== 'ok',
      title: row.name,
      detail: `Host driver ${row.version}`,
      state: row.state === 'ok' ? 'approved' : row.state,
    })),
    ...settings.hooks.rows.map((row) => ({
      mark: '●',
      title: row.provider,
      detail: `Agent hooks · usage collection ${row.usage ? 'on' : 'off'}`,
      state: 'set up',
    })),
  ];
}

/** Why a section has nothing to list: the source could not read it, or there is none. */
function emptyText(section: Section, settings: TmtSettings): string {
  const parts: Part<unknown>[] =
    section === 'drivers'
      ? [settings.drivers, settings.hooks]
      : [settings[section as Exclude<Section, 'drivers'>]];
  const unavailable = parts.map((part) => part.unavailable).find(Boolean);
  if (unavailable) return unavailable;
  return {
    general: 'No settings to show.',
    identities: 'No identities yet.',
    modules: 'No modules.',
    drivers: 'No drivers or agent hooks are set up.',
  }[section];
}

function row(item: SettingsRow): HTMLLIElement {
  const node = document.createElement('li');
  node.className = 'tmt-ui-list-row';
  node.dataset.leading = '';
  const sign = document.createElement('div');
  sign.className = 'tmt-ui-list-leading';
  sign.setAttribute('aria-hidden', 'true');
  sign.textContent = item.mark;
  if (item.waiting) sign.dataset.tone = 'waiting';
  const name = document.createElement('div');
  name.className = 'tmt-ui-list-title';
  if (item.href) {
    const link = document.createElement('a');
    link.href = item.href;
    link.textContent = item.title;
    name.append(link);
  } else name.textContent = item.title;
  const info = document.createElement('div');
  info.className = 'tmt-ui-list-meta';
  const more = document.createElement('div');
  more.className = 'tmt-ui-list-updated';
  more.textContent = item.detail;
  more.title = item.detail;
  const state = document.createElement('div');
  state.className = 'tmt-ui-list-state';
  state.textContent = item.state;
  info.append(more, state);
  node.append(sign, name, info);
  return node;
}

function heading(text: string): HTMLHeadingElement {
  const node = document.createElement('h2');
  node.className = 'home-heading';
  node.textContent = text;
  return node;
}

/** The theme choice: one tab per theme; picking one stores it for this browser and applies it. */
function themeChoice(): HTMLElement {
  const group = document.createElement('div');
  group.className = 'tmt-ui-tab-list';
  group.setAttribute('role', 'tablist');
  group.setAttribute('aria-label', 'Theme');
  let current = storedTheme();
  const buttons = THEMES.map((theme) => {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'tmt-ui-tab';
    button.setAttribute('role', 'tab');
    button.dataset.theme = theme;
    const label = document.createElement('span');
    label.className = 'tmt-ui-tab-label';
    label.textContent = THEME_LABELS[theme];
    button.append(label);
    return button;
  });
  const show = (theme: ThemeChoice) => {
    current = theme;
    for (const button of buttons) {
      const selected = button.dataset.theme === theme;
      button.setAttribute('aria-selected', String(selected));
      button.tabIndex = selected ? 0 : -1;
    }
  };
  const choose = (theme: ThemeChoice) => {
    setStoredTheme(theme);
    show(theme);
  };
  for (const button of buttons) {
    button.addEventListener('click', () => choose(button.dataset.theme as ThemeChoice));
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

/** The section's content. */
export function settingsSection(
  section: Section,
  settings: TmtSettings,
  links?: ReadonlyMap<string, ModuleLink>,
): HTMLElement[] {
  const rows = settingsRows(section, settings, links);
  const body: HTMLElement[] = [];
  if (rows.length) {
    const list = document.createElement('ul');
    list.className = 'tmt-ui-list';
    list.append(...rows.map(row));
    body.push(list);
  } else {
    const none = document.createElement('p');
    none.className = 'home-note';
    none.textContent = emptyText(section, settings);
    body.push(none);
  }
  if (section !== 'general') return body;
  const help = document.createElement('p');
  help.className = 'home-note';
  help.textContent = 'System follows this device. Your choice is kept in this browser only.';
  return [heading('Theme'), themeChoice(), help, heading('Settings'), ...body];
}

export function tmtSettingsPage(source: SettingsSource = settingsSource): void | (() => void) {
  const main = document.getElementById('tmt-settings-section');
  const note = document.getElementById('tmt-settings-note');
  if (!main || !note) return;
  const shell = remoteShell();
  const title = document.querySelector<HTMLElement>('.tmt-ui-title');
  const href = (id: Section) => `${location.pathname}#${id}`;
  const current = (): Section =>
    SECTIONS.find((section) => `#${section.id}` === location.hash)?.id ?? 'general';
  let settings: TmtSettings | undefined;
  let links: Map<string, ModuleLink> = new Map();
  let live = true;
  const paint = () => {
    if (!live) return;
    const section = current();
    shell?.setMenu({
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
    if (settings) main.replaceChildren(...settingsSection(section, settings, links));
  };
  window.addEventListener('hashchange', paint);
  shell?.described((answer) => {
    links = moduleLinks(answer);
    paint();
  });
  paint();
  void source.read().then(
    (next) => {
      if (!live) return;
      settings = next;
      note.textContent = next.sample
        ? 'Sample data: Core has no web read of this machine yet. `tmt ls`, `tmt extension ls` and `tmt driver ls` show the real values.'
        : '';
      note.hidden = !next.sample;
      paint();
    },
    () => {
      if (!live) return;
      note.textContent = 'Settings could not be read.';
      note.hidden = false;
    },
  );
  return () => {
    live = false;
    window.removeEventListener('hashchange', paint);
  };
}
