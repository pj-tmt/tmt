/** The Ops web view, read-only: Agents by squad, Squads, Timers and Ops settings, one section at a
 * time from the module menu. Rows are the shared list rows; every state is a mark and a word. */
import { pageLoading } from './page-loading.js';
import type { RemoteShell } from './remote-shell.js';
import { browserUiClasses, createIcon } from '@tmt/browser-ui/static';
import { notAvailable, notInBrowser } from './not-available.js';
import {
  AGENT_STATES,
  opsSource,
  type AgentMark,
  type AgentState,
  type OpsAgent,
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

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** The agents page's filters: a tab per state, `Idle` covering an idle alert too. */
type Filter = 'all' | 'needs-you' | 'working' | 'idle' | 'blocked' | 'offline' | 'unknown';
const FILTERS: readonly { id: Filter; label: string; states: readonly AgentState[] }[] = [
  { id: 'all', label: 'All', states: [] },
  { id: 'needs-you', label: 'Needs you', states: ['needs-you'] },
  { id: 'working', label: 'Working', states: ['working'] },
  { id: 'idle', label: 'Idle', states: ['idle', 'idle-alert'] },
  { id: 'blocked', label: 'Blocked', states: ['blocked'] },
  { id: 'offline', label: 'Offline', states: ['offline'] },
  { id: 'unknown', label: 'Unknown', states: ['unknown'] },
];

/** What the reader has set on the agents page; it outlives every refresh of the board. */
interface AgentsState {
  filter: Filter;
  query: string;
  collapsed: Set<string>;
  selected?: string;
}
const keyOf = (agent: OpsAgent): string => `${agent.squad}/${agent.name}`;

/** A state's mark, in its own colour. */
function mark(shape: AgentMark, tone?: string): HTMLSpanElement {
  const node = el('span', browserUiClasses.stateMark);
  node.dataset.shape = shape;
  if (tone) node.dataset.tone = tone === 'neutral' ? 'muted' : tone;
  node.setAttribute('aria-hidden', 'true');
  return node;
}

/** A state as a chip: the mark and its word on the state's soft surface. */
function stateChip(state: AgentState, text?: string): HTMLSpanElement {
  const info = AGENT_STATES[state];
  const chip = el('span', browserUiClasses.chip);
  chip.dataset.tone = info.tone;
  chip.append(mark(info.mark), el('span', browserUiClasses.chipLabel, text ?? info.word));
  return chip;
}

const STATS: readonly {
  label: string;
  caption: string;
  tone: string;
  shape: AgentMark;
  states: readonly AgentState[];
}[] = [
  {
    label: 'working',
    caption: 'Actively running',
    tone: 'working',
    shape: 'dot',
    states: ['working'],
  },
  {
    label: 'needs you',
    caption: 'Requires attention',
    tone: 'waiting',
    shape: 'diamond',
    states: ['needs-you'],
  },
  {
    label: 'blocked',
    caption: 'Cannot continue',
    tone: 'blocked',
    shape: 'cross',
    states: ['blocked'],
  },
  {
    label: 'idle',
    caption: 'Waiting for work',
    tone: 'muted',
    shape: 'dashed',
    states: ['idle', 'idle-alert'],
  },
  { label: 'offline', caption: 'Not running', tone: 'muted', shape: 'ring', states: ['offline'] },
  {
    label: 'unknown',
    caption: 'Status unavailable',
    tone: 'muted',
    shape: 'unknown',
    states: ['unknown'],
  },
];

/** One strip of counts: all agents, then each state (idle alert counts as idle). */
function statsStrip(agents: readonly OpsAgent[]): HTMLElement {
  const strip = el('div', browserUiClasses.stats);
  strip.setAttribute('role', 'group');
  strip.setAttribute('aria-label', 'Agents by state');
  const cell = (lead: Node, count: number, label: string, caption: string, tone?: string) => {
    const node = el('div', browserUiClasses.stat);
    if (tone) node.dataset.tone = tone;
    const text = el('div');
    const value = el('p', browserUiClasses.statValue);
    value.append(el('span', browserUiClasses.statNumber, String(count)), ` ${label}`);
    text.append(value, el('p', browserUiClasses.statCaption, caption));
    node.append(lead, text);
    return node;
  };
  strip.append(
    cell(
      createIcon('users'),
      agents.length,
      agents.length === 1 ? 'agent' : 'agents',
      'Total in workspace',
    ),
    ...STATS.map((stat) =>
      cell(
        mark(stat.shape),
        agents.filter((agent) => stat.states.includes(agent.state)).length,
        stat.label,
        stat.caption,
        stat.tone,
      ),
    ),
  );
  return strip;
}

/** The agents by squad in the order the table draws them: squads as the board lists them (then any
 * squad only an agent names), each lead first. */
function bySquad(board: OpsBoard): { name: string; agents: OpsAgent[] }[] {
  const names = [
    ...board.squads.map((squad) => squad.name),
    ...board.agents.map((agent) => agent.squad),
  ].filter((name, at, all) => all.indexOf(name) === at);
  return names
    .map((name) => ({
      name,
      agents: board.agents
        .filter((agent) => agent.squad === name)
        .sort((a, b) => Number(b.role === 'lead') - Number(a.role === 'lead')),
    }))
    .filter((group) => group.agents.length);
}

/** The agents page: the counts, what needs the person, the agents by squad and the chosen one. */
function agentsView(state: AgentsState): { node: HTMLElement; update(board: OpsBoard): void } {
  let board: OpsBoard | undefined;
  const page = el('div', 'ops-agents-page');
  const head = el('header', browserUiClasses.pageHead);
  head.append(
    el('h2', browserUiClasses.pageTitle, 'Your team, in sync.'),
    el('p', browserUiClasses.pageSubtitle, 'A clear view of every squad and agent.'),
  );
  const stats = el('div');
  const banner = el('div');
  const grid = el('div', 'ops-grid');
  const card = el('section', browserUiClasses.card);
  card.dataset.flush = '';
  card.setAttribute('aria-labelledby', 'ops-agents-title');
  const cardHead = el('div', browserUiClasses.cardHead);
  const title = el('h3', browserUiClasses.cardTitle, 'Agents');
  title.id = 'ops-agents-title';
  const tools = el('div', 'ops-agents-tools');
  const tabs = el('div', browserUiClasses.tabs);
  const tabsHeader = el('div', browserUiClasses.tabsHeader);
  const tabList = el('div', browserUiClasses.tabList);
  tabList.setAttribute('role', 'tablist');
  tabList.setAttribute('aria-label', 'Show agents by state');
  const tabButtons = FILTERS.map((filter) => {
    const tab = el('button', browserUiClasses.tab, filter.label);
    tab.type = 'button';
    tab.id = `ops-filter-${filter.id}`;
    tab.setAttribute('role', 'tab');
    tab.setAttribute('aria-controls', 'ops-agents-panel');
    tab.addEventListener('click', () => choose(filter.id));
    tab.addEventListener('keydown', (event) => {
      const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
      if (!step) return;
      event.preventDefault();
      const at = FILTERS.findIndex((item) => item.id === state.filter);
      const next = FILTERS[(at + step + FILTERS.length) % FILTERS.length]!;
      choose(next.id);
      document.getElementById(`ops-filter-${next.id}`)?.focus();
    });
    return tab;
  });
  tabList.append(...tabButtons);
  tabsHeader.append(tabList);
  tabs.append(tabsHeader);
  const search = el('input', browserUiClasses.fieldControl);
  search.type = 'search';
  search.placeholder = 'Filter agents';
  search.setAttribute('aria-label', 'Filter agents');
  search.value = state.query;
  search.addEventListener('input', () => {
    state.query = search.value;
    renderTable();
  });
  const searchField = el('div', browserUiClasses.searchField);
  searchField.append(createIcon('search'), search);
  tools.append(tabs, searchField);
  cardHead.append(title, tools);
  const scroll = el('div', browserUiClasses.tableScroll);
  scroll.id = 'ops-agents-panel';
  scroll.setAttribute('role', 'tabpanel');
  const detail = el('aside', browserUiClasses.card);
  detail.id = 'ops-detail';
  detail.classList.add('ops-detail');
  detail.setAttribute('aria-label', 'Selected agent');
  card.append(cardHead, scroll);
  grid.append(card, detail);
  const foot = el('p', 'ops-foot', 'Read-only · Refreshes while visible');
  page.append(head, stats, banner, grid, foot);

  function choose(filter: Filter): void {
    state.filter = filter;
    renderTable();
  }
  function select(agent: OpsAgent): void {
    state.selected = keyOf(agent);
    renderTable();
    renderDetail();
  }
  const matches = (agent: OpsAgent): boolean => {
    const filter = FILTERS.find((item) => item.id === state.filter)!;
    if (filter.states.length && !filter.states.includes(agent.state)) return false;
    const query = state.query.trim().toLowerCase();
    if (!query) return true;
    return [agent.name, agent.squad, agent.task, agent.next, agent.model, agent.stateText]
      .filter(Boolean)
      .some((value) => value!.toLowerCase().includes(query));
  };
  function renderBanner(): void {
    const waiting = (board?.agents ?? []).filter((agent) => agent.state === 'needs-you');
    banner.replaceChildren();
    if (!waiting.length) return;
    const band = el('section', browserUiClasses.banner);
    band.dataset.tone = 'waiting';
    band.setAttribute('aria-label', 'Needs your attention');
    const bandHead = el('div', browserUiClasses.bannerHead);
    const viewAll = el('button', browserUiClasses.action);
    viewAll.type = 'button';
    viewAll.dataset.variant = 'text';
    viewAll.append(
      el('span', browserUiClasses.actionLabel, 'View all'),
      createIcon('chevron-right'),
    );
    viewAll.addEventListener('click', () => choose('needs-you'));
    bandHead.append(
      createIcon('triangle-alert'),
      el(
        'span',
        undefined,
        `${waiting.length} ${waiting.length === 1 ? 'agent needs' : 'agents need'} your attention`,
      ),
      viewAll,
    );
    band.append(bandHead);
    for (const agent of waiting) {
      const row = el('button', browserUiClasses.bannerRow);
      row.type = 'button';
      row.addEventListener('click', () => {
        state.selected = keyOf(agent);
        renderTable();
        renderDetail();
      });
      const word = el('span');
      word.append(mark('diamond'), AGENT_STATES['needs-you'].word);
      row.append(
        mark('diamond', 'waiting'),
        el('span', undefined, agent.name),
        word,
        el('span', undefined, agent.pending ?? agent.task ?? ''),
        el('span', undefined, agent.next ?? ''),
        createIcon('chevron-right'),
      );
      band.append(row);
    }
    banner.append(band);
  }
  function renderTable(): void {
    for (const [index, filter] of FILTERS.entries()) {
      const on = filter.id === state.filter;
      tabButtons[index]!.setAttribute('aria-selected', String(on));
      tabButtons[index]!.tabIndex = on ? 0 : -1;
    }
    scroll.setAttribute('aria-labelledby', `ops-filter-${state.filter}`);
    if (!board) return;
    const table = el('table', browserUiClasses.table);
    table.dataset.roomy = '';
    const headRow = el('tr');
    for (const text of ['Agent', 'Status', 'Current work', 'Model', 'Waiting on you']) {
      const cell = el('th');
      cell.scope = 'col';
      if (text === 'Model' || text === 'Waiting on you') cell.dataset.compactHide = '';
      if (text === 'Waiting on you') cell.append(el('span', browserUiClasses.visuallyHidden, text));
      else cell.textContent = text;
      headRow.append(cell);
    }
    const thead = el('thead');
    thead.append(headRow);
    table.append(thead);
    let shown = 0;
    for (const { name, agents: all } of bySquad(board)) {
      const agents = all.filter(matches);
      if (!agents.length) continue;
      shown += agents.length;
      const open = !state.collapsed.has(name);
      const body = el('tbody');
      const groupRow = el('tr', browserUiClasses.tableGroup);
      const group = el('th');
      group.colSpan = 5;
      group.scope = 'colgroup';
      const toggle = el('button', browserUiClasses.tableGroupToggle);
      toggle.type = 'button';
      toggle.setAttribute('aria-expanded', String(open));
      toggle.append(
        createIcon('chevron-down'),
        document.createTextNode(name),
        el(
          'span',
          browserUiClasses.tableGroupCount,
          `· ${agents.length} ${agents.length === 1 ? 'agent' : 'agents'}`,
        ),
      );
      toggle.addEventListener('click', () => {
        if (state.collapsed.has(name)) state.collapsed.delete(name);
        else state.collapsed.add(name);
        renderTable();
      });
      group.append(toggle);
      groupRow.append(group);
      body.append(groupRow);
      if (open)
        for (const agent of agents) {
          const info = AGENT_STATES[agent.state];
          const row = el('tr');
          if (keyOf(agent) === state.selected) row.dataset.selected = '';
          const who = el('td');
          const pick = el('button', browserUiClasses.tableSelect);
          pick.type = 'button';
          pick.setAttribute('aria-pressed', String(keyOf(agent) === state.selected));
          pick.append(mark(info.mark, info.tone), document.createTextNode(agent.name));
          pick.addEventListener('click', () => select(agent));
          who.append(pick);
          const status = el('td');
          status.append(stateChip(agent.state));
          const work = el('td', undefined, agent.task || '—');
          // Under the compact width the model and the waiting count move under the work.
          work.append(
            el(
              'span',
              browserUiClasses.tableCompactMeta,
              [agent.model, agent.waitingOnYou > 0 ? `${agent.waitingOnYou} waiting on you` : '']
                .filter(Boolean)
                .join(' · '),
            ),
          );
          const model = el('td', undefined, agent.model || '—');
          model.dataset.compactHide = '';
          const waiting = el('td');
          waiting.dataset.compactHide = '';
          if (agent.waitingOnYou > 0) {
            const count = el('span', browserUiClasses.chip);
            count.dataset.tone = 'waiting';
            count.title = `${agent.waitingOnYou} waiting on you`;
            count.append(el('span', browserUiClasses.chipLabel, String(agent.waitingOnYou)));
            waiting.append(count);
          }
          row.append(who, status, work, model, waiting);
          body.append(row);
        }
      table.append(body);
    }
    scroll.replaceChildren(
      shown
        ? table
        : el('p', 'ops-empty', board.agents.length ? 'No agents match.' : 'No agents yet.'),
    );
  }
  function renderDetail(): void {
    const agent = board?.agents.find((item) => keyOf(item) === state.selected);
    if (!agent) {
      detail.replaceChildren(
        el('h3', browserUiClasses.cardTitle, 'Agent'),
        el('p', 'ops-empty', 'No agents yet.'),
        el('p', 'ops-detail-note', 'Observation only, no agent controls'),
      );
      return;
    }
    const info = AGENT_STATES[agent.state];
    const headline = el('div', browserUiClasses.cardHead);
    const name = el('h3', browserUiClasses.cardTitle);
    name.append(mark(info.mark, info.tone), ' ', agent.name);
    headline.append(name);
    const facts = el('dl', browserUiClasses.facts);
    const fact = (term: string, value: Node | string) => {
      const dd = el('dd');
      dd.append(value);
      facts.append(el('dt', undefined, term), dd);
    };
    fact('Name', agent.name);
    fact('Squad', agent.squad);
    fact('Role', agent.role === 'lead' ? 'Lead' : 'Member');
    fact('Model', agent.model || '—');
    fact('State', stateChip(agent.state, agent.stateText || undefined));
    const block = (term: string, value: string) => {
      const part = el('div', 'ops-detail-block');
      part.append(el('p', 'ops-detail-label', term), el('p', undefined, value));
      return part;
    };
    detail.replaceChildren(
      headline,
      facts,
      block('Task', agent.task || '—'),
      block('Next step', agent.next || '—'),
      ...(agent.pending ? [block('Waiting on you', agent.pending)] : []),
      el('p', 'ops-detail-note', 'Observation only, no agent controls'),
    );
  }
  return {
    node: page,
    update(next) {
      board = next;
      // One agent is always selected, so the panel is never empty: the reader's choice while it is
      // on the board, else the first agent that needs the person, else the first row.
      if (!next.agents.some((agent) => keyOf(agent) === state.selected)) {
        const rows = bySquad(next).flatMap((group) => group.agents);
        const first = rows.find((agent) => agent.state === 'needs-you') ?? rows[0];
        state.selected = first && keyOf(first);
      }
      stats.replaceChildren(statsStrip(next.agents));
      renderBanner();
      renderTable();
      renderDetail();
    },
  };
}

/** The content of the sections other than agents. */
export function opsSection(board: OpsBoard, section: Exclude<Section, 'agents'>): HTMLElement[] {
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
            'clock',
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
  // The agents page keeps its filter, search, collapsed squads and selection across repaints.
  const agents = agentsView({ filter: 'all', query: '', collapsed: new Set() });
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
    if (!board) main.replaceChildren(pageLoading());
    else if (section === 'agents') {
      if (agents.node.parentElement !== main) main.replaceChildren(agents.node);
      agents.update(board);
    } else main.replaceChildren(...opsSection(board, section));
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
