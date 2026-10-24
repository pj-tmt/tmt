/** The pieces the Ops sections share: element and mark helpers, the state chip, the count strip,
 * the page head and the board's squads in display order. */
import { browserUiClasses } from '@tmt/browser-ui/static';
import { createIcon } from '@tmt/browser-ui/glyph';
import { notAvailable } from './not-available.js';
import {
  AGENT_STATES,
  type AgentMark,
  type AgentState,
  type OpsAgent,
  type OpsBoard,
} from './ops-source.js';

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** A sentence in place of a list: why there is nothing to show, with any command as code. */
export function sentence(text: string): HTMLParagraphElement {
  const node = document.createElement('p');
  node.className = 'ops-empty';
  notAvailable(node, text);
  return node;
}

/** A state's mark, in its own colour. */
export function mark(shape: AgentMark, tone?: string): HTMLSpanElement {
  const node = el('span', browserUiClasses.stateMark);
  node.dataset.shape = shape;
  if (tone) node.dataset.tone = tone === 'neutral' ? 'muted' : tone;
  node.setAttribute('aria-hidden', 'true');
  return node;
}

/** A state as a chip: the mark and its word on the state's soft surface. */
export function stateChip(state: AgentState, text?: string): HTMLSpanElement {
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

/** One leading cell of the strip: a total with its own icon. */
export interface TotalStat {
  count: number;
  /** The word after the count, singular or plural. */
  label: string;
  caption: string;
}
export const plural = (count: number, one: string, many: string): string =>
  count === 1 ? one : many;

/** One strip of counts: the totals, then each agent state (idle alert counts as idle). */
export function statsStrip(agents: readonly OpsAgent[], totals: readonly TotalStat[]): HTMLElement {
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
    ...totals.map((total) => cell(createIcon('users'), total.count, total.label, total.caption)),
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

/** The page's headline over its one-line subtitle. */
export function pageHead(title: string, subtitle: string): HTMLElement {
  const head = el('header', browserUiClasses.pageHead);
  head.append(
    el('h2', browserUiClasses.pageTitle, title),
    el('p', browserUiClasses.pageSubtitle, subtitle),
  );
  return head;
}

/** The passive chip a read-only page puts where a control would be. */
export function readOnlyBadge(): HTMLSpanElement {
  const chip = el('span', browserUiClasses.chip);
  chip.append(createIcon('lock'), el('span', browserUiClasses.chipLabel, 'Read only'));
  return chip;
}

/** The line under every Ops page: what the page is and that it keeps itself current. */
export function pageFoot(text = 'Read-only · Refreshes while visible'): HTMLElement {
  const foot = el('p', 'ops-foot');
  foot.append(mark('dot', 'working'), text);
  return foot;
}

/** A card that says what the terminal does for this page: a terminal tile, a title and a sentence,
 * then the one command that opens the Ops workspace with a Copy button that says what happened.
 * `hero` centres it as the page's whole point. */
export function terminalCard(options: {
  id: string;
  title: string;
  text: string;
  hero?: boolean;
}): HTMLElement {
  const card = el('section', browserUiClasses.card);
  if (options.hero) card.dataset.hero = '';
  card.setAttribute('aria-labelledby', options.id);
  const tile = el('span', browserUiClasses.glyphTile);
  tile.append(createIcon('terminal'));
  const title = el('h3', browserUiClasses.cardTitle, options.title);
  title.id = options.id;
  const command = el('span', browserUiClasses.command);
  const copy = el('button', browserUiClasses.commandCopy);
  copy.type = 'button';
  copy.setAttribute('aria-label', 'Copy tmt ops ui');
  copy.append(createIcon('copy'), 'Copy');
  const feedback = el('span', browserUiClasses.commandFeedback);
  feedback.setAttribute('role', 'status');
  copy.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText('tmt ops ui');
      feedback.textContent = 'Copied.';
    } catch {
      feedback.textContent = 'Copy failed. Select the command and copy it manually.';
    }
  });
  command.append(el('code', browserUiClasses.commandText, 'tmt ops ui'), copy, feedback);
  card.append(tile, title, el('p', undefined, options.text), command);
  return card;
}

export const keyOf = (agent: OpsAgent): string => `${agent.squad}/${agent.name}`;

/** The agents by squad in the order the pages draw them: squads as the board lists them (then any
 * squad only an agent names), each lead first; a squad with no agents only when asked for. */
export function bySquad(
  board: OpsBoard,
  keepEmpty = false,
): { name: string; agents: OpsAgent[] }[] {
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
    .filter((group) => keepEmpty || group.agents.length);
}
