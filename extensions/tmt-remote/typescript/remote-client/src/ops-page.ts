/** The Ops web view, read-only: Agents by squad, Squads, Timers and Ops settings, one section at a
 * time from the module menu. Rows are the shared list rows; every state is a mark and a word. */
import { pageLoading } from './page-loading.js';
import type { RemoteShell } from './remote-shell.js';
import { browserUiClasses } from '@tmt/browser-ui/static';
import { notAvailable, notInBrowser } from './not-available.js';
import {
  AGENT_STATES,
  opsSource,
  type AgentMark,
  type AgentState,
  type OpsBoard,
  type OpsSource,
} from './ops-source.js';
import { whileVisible } from './owner-reads.js';

/** The board's own reload cadence (`tmt ops ui`). */
const REFRESH_MS = 5000;

const SECTIONS = [
  { id: 'agents', label: 'Agents' },
  { id: 'squads', label: 'Squads' },
  { id: 'schedules', label: 'Timers' },
  { id: 'settings', label: 'Ops settings' },
] as const;
type Section = (typeof SECTIONS)[number]['id'];

/** One shared list row: the mark, the name over its description, then the note and a state word
 * (toned) in the two columns the shared row keeps for them. */
function row(
  mark: AgentMark,
  tone: string,
  title: string,
  description: string,
  note = '',
  state = '',
): HTMLLIElement {
  const item = document.createElement('li');
  item.className = 'tmt-ui-list-row';
  item.dataset.leading = '';
  const sign = document.createElement('div');
  sign.className = 'tmt-ui-list-leading';
  sign.dataset.tone = tone;
  sign.setAttribute('aria-hidden', 'true');
  const shape = document.createElement('span');
  shape.className = browserUiClasses.stateMark;
  shape.dataset.shape = mark;
  sign.append(shape);
  const name = document.createElement('div');
  name.className = 'tmt-ui-list-title';
  const label = document.createElement('span');
  label.className = 'tmt-ui-list-label';
  label.textContent = title;
  name.append(label);
  if (description) {
    const detail = document.createElement('span');
    detail.className = 'tmt-ui-list-description';
    detail.textContent = description;
    name.append(detail);
  }
  const info = document.createElement('div');
  info.className = 'tmt-ui-list-meta';
  const more = document.createElement('div');
  more.className = 'tmt-ui-list-updated';
  more.textContent = note;
  const word = document.createElement('div');
  word.className = 'tmt-ui-list-state';
  word.dataset.tone = tone;
  word.textContent = state;
  info.append(more, word);
  item.append(sign, name, info);
  return item;
}

function list(rows: HTMLLIElement[]): HTMLUListElement {
  const node = document.createElement('ul');
  node.className = 'tmt-ui-list';
  node.append(...rows);
  return node;
}

/** A sentence in place of a list: why there is nothing to show, with any command as code. */
function sentence(text: string): HTMLParagraphElement {
  const node = document.createElement('p');
  node.className = 'ops-empty';
  notAvailable(node, text);
  return node;
}

/** The board by urgency, so what needs the person shows before any row. Idle is the normal
 * state and has no tile; a tile at 0 stays, muted, so the row keeps its shape. */
function summary(board: OpsBoard): HTMLDListElement {
  const tiles: [string, string, readonly AgentState[]][] = [
    ['Needs you', 'waiting', ['needs-you']],
    ['Blocked', 'blocked', ['blocked']],
    ['Offline', 'muted', ['offline']],
    ['Working', 'working', ['working']],
  ];
  const node = document.createElement('dl');
  node.className = browserUiClasses.statusTiles;
  for (const [label, tone, states] of tiles) {
    const count = board.agents.filter((agent) => states.includes(agent.state)).length;
    const tile = document.createElement('div');
    tile.className = browserUiClasses.statusTile;
    tile.dataset.tone = count === 0 ? 'muted' : tone;
    const term = document.createElement('dt');
    term.className = browserUiClasses.statusTileLabel;
    term.textContent = label;
    const value = document.createElement('dd');
    value.className = browserUiClasses.statusTileValue;
    value.dataset.numeric = count === 0 ? 'zero' : 'count';
    const dot = document.createElement('span');
    dot.className = browserUiClasses.statusTileDot;
    dot.setAttribute('aria-hidden', 'true');
    value.append(dot, String(count));
    tile.append(term, value);
    node.append(tile);
  }
  return node;
}

function heading(text: string): HTMLHeadingElement {
  const node = document.createElement('h2');
  node.className = browserUiClasses.sectionHeading;
  node.textContent = text;
  return node;
}

/** The section's content; agents group under their squad, the lead first. */
export function opsSection(board: OpsBoard, section: Section): HTMLElement[] {
  if (section === 'agents')
    return [
      summary(board),
      ...board.squads.flatMap((squad) => {
        const agents = board.agents
          .filter((agent) => agent.squad === squad.name)
          .sort((a, b) => Number(b.role === 'lead') - Number(a.role === 'lead'));
        if (!agents.length) return [];
        return [
          heading(squad.lead ? `${squad.name} · lead ${squad.lead}` : squad.name),
          list(
            agents.map((agent) => {
              const state = AGENT_STATES[agent.state];
              const detail = [
                agent.stateText && agent.stateText !== state.word ? agent.stateText : '',
                agent.pending ? `Waiting on you: ${agent.pending}` : '',
                agent.waitingOnYou > 0 ? `${agent.waitingOnYou} waiting` : '',
                agent.task ?? '',
                agent.next ? `next ${agent.next}` : '',
              ].filter(Boolean);
              return row(
                state.mark,
                state.tone,
                agent.name,
                detail.join(' · '),
                agent.model ?? '',
                state.word,
              );
            }),
          ),
        ];
      }),
    ];
  if (section === 'squads')
    return [
      list(
        board.squads.map((squad) => {
          const count = board.agents.filter((agent) => agent.squad === squad.name).length;
          return row(
            'dot',
            'neutral',
            squad.name,
            `${squad.lead ? `lead ${squad.lead}` : ''}${
              squad.idleAlertMinutes
                ? `${squad.lead ? ' · ' : ''}idle alert ${squad.idleAlertMinutes}m`
                : ''
            }`,
            `${count} agent${count === 1 ? '' : 's'}`,
          );
        }),
      ),
    ];
  if (section === 'schedules') {
    if (board.schedules === null)
      return [sentence('Your machine could not read the timers. Run `tmt ops ui` to see them.')];
    if (!board.schedules.length) return [sentence('No timers are set.')];
    return [
      list(
        board.schedules.map((schedule) =>
          row(
            'target',
            'neutral',
            schedule.what || schedule.id,
            [schedule.squad, schedule.owner].filter(Boolean).join(' · '),
            schedule.when,
            schedule.id,
          ),
        ),
      ),
    ];
  }
  // The board's settings have no browser read: the page says where they are.
  return [sentence(notInBrowser('The Ops settings page', 'tmt ops ui'))];
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
    main.replaceChildren(...(board ? opsSection(board, section) : [pageLoading()]));
  };
  window.addEventListener('hashchange', paint, { signal: shell.signal });
  paint();
  // The board while the route is visible, at the board's own cadence; a page repaints only when
  // the board changed, so a reader's place and selection stay.
  let last = '';
  whileVisible(REFRESH_MS, shell.signal, async () => {
    const next = await source.board(shell.signal);
    if (shell.signal.aborted) return true;
    const seen = JSON.stringify(next);
    if (seen !== last) {
      last = seen;
      if (next.unavailable) {
        board = undefined;
        main.replaceChildren();
        notAvailable(note, next.unavailable);
      } else {
        board = next;
        if (next.truncated)
          notAvailable(
            note,
            'The board is larger than the browser shows. `tmt ops ui` shows all of it.',
          );
        else {
          note.textContent = '';
          note.hidden = true;
        }
        paint();
      }
    }
    return next.unavailable === undefined;
  });
}
