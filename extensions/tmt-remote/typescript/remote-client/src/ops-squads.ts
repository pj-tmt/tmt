/** The squads page: each squad as a card with its lead, its people and its idle alert. */
import { avatarInitials, browserUiClasses } from '@tmt/browser-ui/static';
import { createIcon } from '@tmt/browser-ui/glyph';
import { AGENT_STATES, type OpsAgent, type OpsBoard, type OpsSquad } from './ops-source.js';
import { agentDrawer } from './ops-drawer.js';
import {
  bySquad,
  el,
  keyOf,
  mark,
  pageFoot,
  pageHead,
  plural,
  readOnlyBadge,
  stateChip,
  statsStrip,
} from './ops-parts.js';

const minutes = (count: number): string => `${count} ${plural(count, 'minute', 'minutes')}`;
const idleAlert = (squad: OpsSquad | undefined): string =>
  squad?.idleAlertMinutes ? `after ${minutes(squad.idleAlertMinutes)}` : 'not configured';

export function squadsView(): { node: HTMLElement; update(board: OpsBoard): void; close(): void } {
  const members = new Map<string, HTMLButtonElement>();
  const heading = pageHead(
    'Squads, clearly connected.',
    'Every team, its lead and the people moving work forward.',
  );
  const fallback = heading.querySelector('h2')!;
  fallback.tabIndex = -1;
  const drawer = agentDrawer({
    returnFocus: (agent) => members.get(keyOf(agent)),
    fallback: () => fallback,
  });
  const page = el('div', 'ops-squads-page');
  const stats = el('div');
  const cards = el('div', 'ops-squad-cards');
  page.append(heading, stats, cards, pageFoot(), drawer.node);

  function member(agent: OpsAgent): HTMLButtonElement {
    const button = el('button', 'ops-squad-member');
    button.type = 'button';
    button.setAttribute('aria-label', `${agent.name}, ${AGENT_STATES[agent.state].word}`);
    button.append(
      mark(AGENT_STATES[agent.state].mark, AGENT_STATES[agent.state].tone),
      el('strong', undefined, agent.name),
      stateChip(agent.state),
    );
    button.addEventListener('click', () => drawer.open(agent));
    members.set(keyOf(agent), button);
    return button;
  }

  return {
    node: page,
    close: drawer.close,
    update(board) {
      members.clear();
      const groups = bySquad(board, true);
      stats.replaceChildren(
        statsStrip(board.agents, [
          {
            count: board.agents.length,
            label: plural(board.agents.length, 'agent', 'agents'),
            caption: 'Total in workspace',
          },
        ]),
      );
      cards.replaceChildren(
        ...groups.map((group) => {
          const squad = board.squads.find((item) => item.name === group.name);
          const card = el('section', `${browserUiClasses.card} ops-squad-card`);
          card.setAttribute('aria-label', group.name);
          const top = el('div', 'ops-squad-top');
          const names = el('div');
          names.append(
            el('h3', browserUiClasses.cardTitle, group.name),
            el(
              'p',
              'ops-squad-meta',
              [
                squad?.lead ? `Lead ${squad.lead}` : '',
                `${group.agents.length} ${plural(group.agents.length, 'agent', 'agents')}`,
              ]
                .filter(Boolean)
                .join(' · '),
            ),
          );
          const people = el('div', browserUiClasses.avatarStack);
          people.setAttribute('aria-hidden', 'true');
          for (const agent of group.agents) {
            const avatar = el('span', browserUiClasses.avatar, avatarInitials(agent.name));
            avatar.dataset.size = 's';
            people.append(avatar);
          }
          top.append(names, people);
          const members = el('div', 'ops-squad-members');
          if (group.agents.length) members.append(...group.agents.map(member));
          else members.append(el('p', 'ops-empty', 'No agents in this squad.'));
          const bottom = el('div', 'ops-squad-bottom');
          bottom.append(
            createIcon('clock'),
            el('span', undefined, `Idle alert ${idleAlert(squad)}`),
            readOnlyBadge(),
          );
          card.append(top, members, bottom);
          return card;
        }),
      );
      if (!groups.length) cards.append(el('p', 'ops-empty', 'No squads yet.'));
      drawer.update(board.agents);
    },
  };
}
