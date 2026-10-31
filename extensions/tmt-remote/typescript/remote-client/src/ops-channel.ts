/** Local channel / DM read panels. The board remains the only source; history and all writes
 * are explicitly unavailable until their separately admitted operations ship. */
import { browserUiClasses as c } from '@tmt/browser-ui/static';
import type { OpsAgent, OpsBoard } from './ops-source.js';
import { el, pageFoot } from './ops-parts.js';
import { opsHref, type OpsPlace } from './ops-menu.js';
import { commandCopy } from './command-copy.js';

type ChannelPlace = Exclude<OpsPlace, string>;
export function memberState(agent: Pick<OpsAgent, 'presence' | 'activity'>): string {
  if (agent.presence === 'offline') return 'Offline';
  if (agent.activity?.state === 'working') return 'Working';
  if (agent.activity?.state === 'idle') return 'Idle';
  return 'Unknown';
}

export function channelView(pathname: string): {
  node: HTMLElement;
  update(board: OpsBoard, place: ChannelPlace): void;
} {
  const page = el('div', 'ops-channel');
  const context = el('p', c.pageSubtitle, 'This machine · Read only');
  const panel = el('details', `${c.disclosure} ${c.tabs} ops-channel-context`);
  panel.open = true;
  panel.dataset.variant = 'flat';
  panel.dataset.presentation = 'compact';
  const summary = el('summary');
  const tabs = el('div', c.tabList);
  tabs.setAttribute('role', 'tablist');
  tabs.setAttribute('aria-label', 'Channel details');
  const facts = el('section', c.tabPanel);
  facts.id = 'ops-channel-about';
  facts.setAttribute('role', 'tabpanel');
  const timers = el('section', c.tabPanel);
  timers.id = 'ops-channel-timers';
  timers.setAttribute('role', 'tabpanel');
  let selected = 0;
  const buttons = [0, 1].map((index) => {
    const button = el('button', c.tab);
    button.type = 'button';
    button.id = `ops-channel-tab-${index}`;
    button.setAttribute('role', 'tab');
    button.setAttribute('aria-controls', index ? timers.id : facts.id);
    button.addEventListener('click', () => select(index));
    button.addEventListener('keydown', (event) => {
      if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
      event.preventDefault();
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? 1 : 1 - index;
      select(next);
      buttons[next]!.focus();
    });
    return button;
  });
  function select(index: number): void {
    selected = index;
    buttons.forEach((button, at) => {
      button.tabIndex = at === index ? 0 : -1;
      button.setAttribute('aria-selected', String(at === index));
    });
    facts.hidden = index !== 0;
    timers.hidden = index !== 1;
  }
  facts.setAttribute('aria-labelledby', buttons[0]!.id);
  timers.setAttribute('aria-labelledby', buttons[1]!.id);
  tabs.append(...buttons);
  panel.append(summary, tabs, facts, timers);
  const history = el('section', 'ops-channel-history');
  const unavailable = el('h2', c.sectionHeading);
  const explanation = el('p');
  history.append(unavailable, explanation, commandCopy('tmt ops ui'));
  const compose = el('section', 'ops-channel-compose');
  const reason = el('p', 'ops-detail-note');
  reason.id = 'ops-channel-write-reason';
  const actions = el('div', c.actions);
  for (const label of ['Send', 'New timer', 'Edit idle reminder']) {
    const button = el('button', c.action, label);
    button.type = 'button';
    button.disabled = true;
    button.setAttribute('aria-describedby', reason.id);
    actions.append(button);
  }
  compose.append(actions, reason);
  page.append(context, panel, history, compose, pageFoot());
  select(0);
  let identity = '';
  let fingerprint = '';
  const memberList = el('ul', c.list);
  const memberRows = new Map<string, HTMLElement>();

  function memberRow(agent: OpsAgent): HTMLElement {
    const row = el('li', c.listRow);
    const title = el('div', c.listTitle);
    const name = el('a', undefined, agent.name);
    title.append(name);
    name.href = opsHref(pathname, { squad: agent.squad, memberId: agent.id });
    const detail = el('div', c.listMeta);
    detail.append(
      el('span', c.listState, memberState(agent)),
      el('span', c.listUpdated, agent.role === 'lead' ? 'Lead' : 'Member'),
    );
    row.append(title, detail);
    return row;
  }
  return {
    node: page,
    update(board, place) {
      const key = opsHref(pathname, place);
      const people = board.agents
        .filter((agent) => agent.squad === place.squad)
        .sort((a, b) => Number(b.role === 'lead') - Number(a.role === 'lead'));
      const member = place.memberId
        ? people.find((agent) => agent.id === place.memberId)
        : undefined;
      const squad = board.squads.find((squad) => squad.name === place.squad);
      const known = place.memberId ? !!member : !!squad || people.length > 0;
      const jobs = board.schedules?.filter((job) => job.squad === place.squad);
      const seen = JSON.stringify({ key, people, squad, jobs, known, truncated: board.truncated });
      if (seen === fingerprint) return;
      fingerprint = seen;
      if (identity !== key) {
        identity = key;
        memberRows.clear();
        memberList.replaceChildren();
        panel.open = true;
        select(0);
      }
      context.textContent = `This machine · ${place.squad} · Read only`;
      panel.hidden = !known;
      history.hidden = !known;
      compose.hidden = !known;
      let missing = page.querySelector<HTMLElement>('[data-ops-channel-missing]');
      if (!known) {
        missing ??= el('p', 'ops-empty');
        missing.dataset.opsChannelMissing = '';
        missing.textContent = board.truncated
          ? 'This selection is not in the partial board. Open Ops in the terminal to see the full list.'
          : place.memberId
            ? 'This member is no longer listed in this squad. Choose a current member from the sidebar.'
            : 'This squad is no longer listed. Choose a current squad from the sidebar.';
        if (!missing.isConnected) page.append(missing);
        // Hidden summaries must drop removed identities and old timer counts too.
        summary.textContent = 'Details unavailable';
        memberRows.clear();
        memberList.replaceChildren();
        facts.replaceChildren();
        timers.replaceChildren();
        return;
      }
      missing?.remove();
      const timerCount =
        jobs && !board.truncated ? ` · Timers ${jobs.length}` : ' · Timers unavailable';
      summary.textContent = place.memberId
        ? `About ${member!.name}${timerCount}`
        : `Members ${board.truncated ? '(partial)' : people.length}${timerCount}`;
      buttons[0]!.textContent = place.memberId ? 'About' : 'Members';
      buttons[1]!.textContent = 'Squad timers';
      tabs.setAttribute('aria-label', place.memberId ? 'Member details' : 'Channel details');
      if (member) {
        const details = el('dl', c.facts);
        for (const [term, value] of [
          ['Name', member.name],
          ['Squad', member.squad],
          ['Role', member.role === 'lead' ? 'Lead' : 'Member'],
          ['State', memberState(member)],
          ['Model', member.model || 'Not reported'],
          ['Task', member.task || 'Not reported'],
          ['Next step', member.next || 'Not reported'],
        ])
          details.append(el('dt', undefined, term), el('dd', undefined, value));
        facts.replaceChildren(details);
      } else {
        const ids = new Set(people.map((agent) => agent.id));
        for (const [id, row] of memberRows)
          if (!ids.has(id)) {
            const heldFocus = row.contains(document.activeElement);
            row.remove();
            memberRows.delete(id);
            if (heldFocus) summary.focus();
          }
        people.forEach((agent, index) => {
          let row = memberRows.get(agent.id);
          if (!row) {
            row = memberRow(agent);
            memberRows.set(agent.id, row);
          }
          row.querySelector('a')!.textContent = agent.name;
          row.querySelector(`.${c.listState}`)!.textContent = memberState(agent);
          row.querySelector(`.${c.listUpdated}`)!.textContent =
            agent.role === 'lead' ? 'Lead' : 'Member';
          if (memberList.children[index] !== row)
            memberList.insertBefore(row, memberList.children[index] ?? null);
        });
        if (people.length) {
          if (memberList.parentElement !== facts) facts.replaceChildren(memberList);
        } else facts.replaceChildren(el('p', 'ops-empty', 'No members listed.'));
      }
      const timerRows = el('ul', c.list);
      for (const job of jobs ?? []) {
        const row = el('li', c.listRow);
        const meta = el('div', c.listMeta);
        meta.append(el('span', c.listUpdated, job.when));
        row.append(el('span', c.listTitle, job.what || job.id), meta);
        timerRows.append(row);
      }
      timers.replaceChildren(
        el('p', 'ops-detail-note', 'Squad timers · Shown for the whole squad.'),
        jobs === undefined
          ? el('p', 'ops-empty', 'Squad timers are unavailable.')
          : jobs.length
            ? timerRows
            : el(
                'p',
                'ops-empty',
                board.truncated
                  ? 'No squad timers in this partial read.'
                  : 'No squad timers are set.',
              ),
        ...(board.truncated
          ? [el('p', 'ops-detail-note', 'Partial board; totals are not known.')]
          : []),
      );
      unavailable.textContent = place.memberId
        ? "History isn't available in the browser yet"
        : "Channel messages aren't available in the browser yet";
      explanation.textContent = place.memberId
        ? 'Open Ops in this machine’s terminal, choose this member and open Talk to read the retained conversation.'
        : 'This browser shows members and squad timers. Channel messages and threads are not available yet.';
      reason.textContent =
        'Sending, timer changes and idle-reminder changes are not available in the browser yet. Use this machine’s terminal.';
      select(selected);
    },
  };
}
