/** The Ops web view, read-only: Agents by squad, Squads, Timers and Ops settings, one section at a
 * time from the module menu. Rows are the shared list rows; every state is a mark and a word. */
import { pageLoading } from './page-loading.js';
import type { RemoteShell } from './remote-shell.js';
import { notAvailable, notInBrowser } from './not-available.js';
import { AGENT_STATES, opsSource, type OpsBoard, type OpsSource } from './ops-source.js';
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

/** A sentence in place of a list: why there is nothing to show, with any command as code. */
function sentence(text: string): HTMLParagraphElement {
  const node = document.createElement('p');
  node.className = 'ops-empty';
  notAvailable(node, text);
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
        heading(squad.lead ? `${squad.name} · lead ${squad.lead}` : squad.name),
        list(
          agents.map((agent) => {
            const state = AGENT_STATES[agent.state];
            const detail = [
              agent.stateText ?? state.word,
              agent.pending ? `Waiting on you: ${agent.pending}` : '',
              agent.waitingOnYou > 0 ? `${agent.waitingOnYou} waiting` : '',
              agent.task ?? '',
              agent.next ? `next ${agent.next}` : '',
            ].filter(Boolean);
            return row(state.mark, state.tone, agent.name, detail.join(' · '), agent.model ?? '');
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
            `${squad.lead ? `lead ${squad.lead} · ` : ''}${count} agent${count === 1 ? '' : 's'}` +
              (squad.idleAlertMinutes ? ` · idle alert ${squad.idleAlertMinutes}m` : ''),
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
            '◷',
            'neutral',
            schedule.what || schedule.id,
            [schedule.squad, schedule.when, schedule.owner].filter(Boolean).join(' · '),
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
