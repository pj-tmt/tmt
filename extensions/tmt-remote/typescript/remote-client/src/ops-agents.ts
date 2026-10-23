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
      stats.replaceChildren(
        statsStrip(next.agents, [
          {
            count: next.agents.length,
            label: plural(next.agents.length, 'agent', 'agents'),
            caption: 'Total in workspace',
          },
        ]),
      );
      renderBanner();
      renderTable();
      renderDetail();
    },
  };
}
