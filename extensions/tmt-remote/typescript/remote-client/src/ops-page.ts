/** The Ops web view, read-only: Agents by squad, Squads, Timers and Ops settings, one section at a
 * time from the module menu. Rows are the shared list rows; every state is a mark and a word. */
import type { RemoteShell } from './remote-shell.js';
import { AGENT_STATES, opsSource, type OpsBoard, type OpsSource } from './ops-source.js';

const SECTIONS = [
  { id: 'agents', label: 'Agents' },
  { id: 'squads', label: 'Squads' },
  { id: 'schedules', label: 'Timers' },
  { id: 'settings', label: 'Ops settings' },
] as const;
type Section = (typeof SECTIONS)[number]['id'];

function row(mark: string, tone: string, title: string, detail: string, meta = ''): HTMLLIElement {
  const item = document.createElement('li');
  item.className = 'tmt-ui-list-row';
  item.dataset.leading = '';
  const sign = document.createElement('div');
  sign.className = 'tmt-ui-list-leading ops-mark';
  sign.dataset.tone = tone;
  sign.setAttribute('aria-hidden', 'true');
  sign.textContent = mark;
  const name = document.createElement('div');
  name.className = 'tmt-ui-list-title';
  name.textContent = title;
  const info = document.createElement('div');
  info.className = 'tmt-ui-list-meta';
  const more = document.createElement('div');
  more.className = 'tmt-ui-list-updated';
  more.textContent = detail;
  more.title = detail;
  const state = document.createElement('div');
  state.className = 'tmt-ui-list-state';
  state.textContent = meta;
  info.append(more, state);
  item.append(sign, name, info);
  return item;
}

function list(rows: HTMLLIElement[]): HTMLUListElement {
  const node = document.createElement('ul');
  node.className = 'tmt-ui-list';
  node.append(...rows);
  return node;
}

function heading(text: string): HTMLHeadingElement {
  const node = document.createElement('h2');
  node.className = 'home-heading';
  node.textContent = text;
  return node;
}

/** The section's content; agents group under their squad, the lead first. */
export function opsSection(board: OpsBoard, section: Section): HTMLElement[] {
  if (section === 'agents')
    return board.squads.flatMap((squad) => {
      const agents = board.agents
        .filter((agent) => agent.squad === squad.name)
        .sort((a, b) => Number(b.role === 'lead') - Number(a.role === 'lead'));
      if (!agents.length) return [];
      return [
        heading(`${squad.name} · lead ${squad.lead}`),
        list(
          agents.map((agent) => {
            const state = AGENT_STATES[agent.state];
            return row(
              state.mark,
              state.tone,
              agent.name,
              `${state.word}${agent.task ? ` · ${agent.task}` : ''}`,
              agent.model ?? '',
            );
          }),
        ),
      ];
    });
  if (section === 'squads')
    return [
      list(
        board.squads.map((squad) => {
          const count = board.agents.filter((agent) => agent.squad === squad.name).length;
          return row(
            '●',
            'neutral',
            squad.name,
            `lead ${squad.lead} · ${count} agent${count === 1 ? '' : 's'}` +
              (squad.idleAlertMinutes ? ` · idle alert ${squad.idleAlertMinutes}m` : ''),
          );
        }),
      ),
    ];
  if (section === 'schedules')
    return [
      list(
        board.schedules.map((schedule) =>
          row('◷', 'neutral', schedule.what, `${schedule.squad} · ${schedule.when}`, schedule.id),
        ),
      ),
    ];
  return [
    list(
      board.settings.map((setting) =>
        row('', 'neutral', setting.name, setting.description, setting.value),
      ),
    ),
  ];
}

export function opsPage(shell: RemoteShell, source: OpsSource = opsSource): void {
  const main = document.getElementById('ops-section');
  const title = document.querySelector<HTMLElement>('.tmt-ui-title');
  const note = document.getElementById('ops-note');
  if (!main || !note) return;
  const href = (id: Section) => `${location.pathname}#${id}`;
  const current = (): Section =>
    SECTIONS.find((section) => `#${section.id}` === location.hash)?.id ?? 'agents';
  let board: OpsBoard | undefined;
  const paint = () => {
    const section = current();
    shell.setMenu({
      title: 'Ops',
      groups: [
        {
          items: SECTIONS.filter((item) => item.id !== 'settings').map((item) => ({
            id: item.id,
            label: item.label,
            href: href(item.id),
            current: item.id === section,
          })),
        },
        {
          items: [
            {
              id: 'settings',
              label: 'Ops settings',
              href: href('settings'),
              current: section === 'settings',
            },
          ],
        },
      ],
    });
    if (title) title.textContent = SECTIONS.find((item) => item.id === section)!.label;
    if (board) main.replaceChildren(...opsSection(board, section));
  };
  window.addEventListener('hashchange', paint);
  paint();
  void source.board().then((next) => {
    board = next;
    note.textContent = next.sample
      ? 'Sample data: Ops has no web read of your board yet. `tmt ops ui` shows the real board.'
      : '';
    note.hidden = !next.sample;
    paint();
  });
}
