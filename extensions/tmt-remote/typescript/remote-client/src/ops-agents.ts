/** The agents page: the counts, what needs the person, the agents by squad and the chosen one. */
import { browserUiClasses } from '@tmt/browser-ui/static';
import { createIcon } from '@tmt/browser-ui/glyph';
import { AGENT_STATES, type AgentState, type OpsAgent, type OpsBoard } from './ops-source.js';
import { bySquad, el, keyOf, mark, pageHead, plural, stateChip, statsStrip } from './ops-parts.js';

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

/** How long typing in the search box waits before the table filters; clearing and Enter apply at once. */
const SEARCH_DEBOUNCE_MS = 200;

/** Puts exactly `wanted` under `parent` in order, moving only nodes that are out of place, so a
 * node that stays keeps its focus, text selection and scroll. */
function reconcile(parent: Node, wanted: readonly Node[]): void {
  // Leaving nodes go first, so they never stand in front of a kept one and force it to move.
  const keep = new Set(wanted);
  for (let child = parent.firstChild; child;) {
    const next = child.nextSibling;
    if (!keep.has(child)) child.remove();
    child = next;
  }
  wanted.forEach((node, index) => {
    const at = parent.childNodes[index];
    if (at !== node) parent.insertBefore(node, at ?? null);
  });
}

/** What the reader has set on the agents page; it outlives every refresh of the board. */
interface AgentsState {
  filter: Filter;
  query: string;
  collapsed: Set<string>;
  selected?: string;
}

export function agentsView(): { node: HTMLElement; update(board: OpsBoard): void } {
  const state: AgentsState = { filter: 'all', query: '', collapsed: new Set() };
  let board: OpsBoard | undefined;
  const page = el('div', 'ops-agents-page');
  const head = pageHead('Your team, in sync.', 'A clear view of every squad and agent.');
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
  let typing: ReturnType<typeof setTimeout> | undefined;
  const applySearch = () => {
    clearTimeout(typing);
    typing = undefined;
    if (state.query === search.value) return;
    state.query = search.value;
    renderTable();
  };
  search.addEventListener('input', () => {
    clearTimeout(typing);
    if (search.value) typing = setTimeout(applySearch, SEARCH_DEBOUNCE_MS);
    else applySearch();
  });
  search.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') applySearch();
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
  function select(key: string): void {
    state.selected = key;
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
  let bannerShown = '';
  function renderBanner(): void {
    const waiting = (board?.agents ?? []).filter((agent) => agent.state === 'needs-you');
    const shown = JSON.stringify(
      waiting.map((agent) => [keyOf(agent), agent.name, agent.pending, agent.task, agent.next]),
    );
    if (shown === bannerShown) return;
    bannerShown = shown;
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
      const key = keyOf(agent);
      row.addEventListener('click', () => select(key));
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
  // The table, its head and each squad's group are built once and kept; each agent's row is kept
  // by its key, and a refresh replaces only the cells whose inputs changed.
  const table = el('table', browserUiClasses.table);
  table.dataset.roomy = '';
  const thead = el('thead');
  const headRow = el('tr');
  for (const text of ['Agent', 'Status', 'Current work', 'Model', 'Waiting on you']) {
    const cell = el('th');
    cell.scope = 'col';
    if (text === 'Model' || text === 'Waiting on you') cell.dataset.compactHide = '';
    if (text === 'Waiting on you') cell.append(el('span', browserUiClasses.visuallyHidden, text));
    else cell.textContent = text;
    headRow.append(cell);
  }
  thead.append(headRow);
  table.append(thead);
  const empty = el('p', 'ops-empty');
  let groups = new Map<
    string,
    { body: HTMLTableSectionElement; head: HTMLTableRowElement; shown: string }
  >();
  let rows = new Map<string, { row: HTMLTableRowElement; shown: string[] }>();
  // The rows the render in progress keeps; it replaces `rows` when the render ends.
  let nextRows = new Map<string, { row: HTMLTableRowElement; shown: string[] }>();
  function groupHead(name: string, count: number, open: boolean): HTMLTableRowElement {
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
        `· ${count} ${count === 1 ? 'agent' : 'agents'}`,
      ),
    );
    toggle.addEventListener('click', () => {
      if (state.collapsed.has(name)) state.collapsed.delete(name);
      else state.collapsed.add(name);
      renderTable();
    });
    group.append(toggle);
    groupRow.append(group);
    return groupRow;
  }
  /** The row's five cells, each with the inputs it shows; equal inputs keep the cell. */
  function agentCells(agent: OpsAgent): [string, () => HTMLElement][] {
    const info = AGENT_STATES[agent.state];
    const key = keyOf(agent);
    return [
      [
        JSON.stringify([agent.name, agent.state]),
        () => {
          const who = el('td');
          const pick = el('button', browserUiClasses.tableSelect);
          pick.type = 'button';
          pick.append(mark(info.mark, info.tone), document.createTextNode(agent.name));
          pick.addEventListener('click', () => select(key));
          who.append(pick);
          return who;
        },
      ],
      [
        JSON.stringify([agent.state]),
        () => {
          const status = el('td');
          status.append(stateChip(agent.state));
          return status;
        },
      ],
      [
        JSON.stringify([agent.task, agent.model, agent.waitingOnYou]),
        () => {
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
          return work;
        },
      ],
      [
        JSON.stringify([agent.model]),
        () => {
          const model = el('td', undefined, agent.model || '—');
          model.dataset.compactHide = '';
          return model;
        },
      ],
      [
        JSON.stringify([agent.waitingOnYou]),
        () => {
          const waiting = el('td');
          waiting.dataset.compactHide = '';
          if (agent.waitingOnYou > 0) {
            const count = el('span', browserUiClasses.chip);
            count.dataset.tone = 'waiting';
            count.title = `${agent.waitingOnYou} waiting on you`;
            count.append(el('span', browserUiClasses.chipLabel, String(agent.waitingOnYou)));
            waiting.append(count);
          }
          return waiting;
        },
      ],
    ];
  }
  function agentRow(agent: OpsAgent): HTMLTableRowElement {
    const key = keyOf(agent);
    const selected = key === state.selected;
    const cells = agentCells(agent);
    const kept = rows.get(key);
    const row = kept?.row ?? el('tr');
    if (selected) row.dataset.selected = '';
    else delete row.dataset.selected;
    const shown = cells.map(([inputs], index) => {
      const old = row.children[index];
      if (!old || kept?.shown[index] !== inputs) {
        const cell = cells[index]![1]();
        if (old) old.replaceWith(cell);
        else row.append(cell);
      }
      return inputs;
    });
    // Selection changes in place, so the button the reader just pressed keeps its focus.
    row.querySelector('button')?.setAttribute('aria-pressed', String(selected));
    nextRows.set(key, { row, shown });
    return row;
  }
  function renderTable(): void {
    for (const [index, filter] of FILTERS.entries()) {
      const on = filter.id === state.filter;
      tabButtons[index]!.setAttribute('aria-selected', String(on));
      tabButtons[index]!.tabIndex = on ? 0 : -1;
    }
    scroll.setAttribute('aria-labelledby', `ops-filter-${state.filter}`);
    if (!board) return;
    nextRows = new Map();
    const nextGroups = new Map<
      string,
      { body: HTMLTableSectionElement; head: HTMLTableRowElement; shown: string }
    >();
    const bodies: HTMLTableSectionElement[] = [];
    let shown = 0;
    for (const { name, agents: all } of bySquad(board)) {
      const agents = all.filter(matches);
      if (!agents.length) continue;
      shown += agents.length;
      const open = !state.collapsed.has(name);
      const inputs = JSON.stringify([agents.length, open]);
      const kept = groups.get(name);
      const body = kept?.body ?? el('tbody');
      const head = kept && kept.shown === inputs ? kept.head : groupHead(name, agents.length, open);
      nextGroups.set(name, { body, head, shown: inputs });
      reconcile(body, [head, ...(open ? agents.map(agentRow) : [])]);
      bodies.push(body);
    }
    groups = nextGroups;
    rows = nextRows;
    reconcile(table, [thead, ...bodies]);
    if (shown) reconcile(scroll, [table]);
    else {
      empty.textContent = board.agents.length ? 'No agents match.' : 'No agents yet.';
      reconcile(scroll, [empty]);
    }
  }
  let detailShown: string | undefined;
  function renderDetail(): void {
    const agent = board?.agents.find((item) => keyOf(item) === state.selected);
    const inputs = JSON.stringify(agent ?? null);
    if (inputs === detailShown) return;
    detailShown = inputs;
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
  let statsShown: string | undefined;
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
      const counted = JSON.stringify(next.agents.map((agent) => agent.state));
      if (counted !== statsShown) {
        statsShown = counted;
        stats.replaceChildren(
          statsStrip(next.agents, [
            {
              count: next.agents.length,
              label: plural(next.agents.length, 'agent', 'agents'),
              caption: 'Total in workspace',
            },
          ]),
        );
      }
      renderBanner();
      renderTable();
      renderDetail();
    },
  };
}
