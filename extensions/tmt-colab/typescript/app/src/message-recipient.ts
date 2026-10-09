import type { AgentDestination } from './live-ask.js';
import type { ComposerEdit } from './components/message-composer-edit.js';

export type RecipientKey = Pick<AgentDestination, 'machine' | 'agent'>;
export const MESSAGE_RECIPIENT_LIMIT = 8;
export function recipientId(key: RecipientKey) {
  return `${key.machine}:${key.agent}`;
}
export type MentionResolution = {
  mentions: NonNullable<ComposerEdit['mentions']>;
  unknown: string[];
  ambiguous: string[];
  ambiguousTokens: { name: string; range: { start: number; end: number } }[];
  unresolved: boolean;
};
const boundary = /[\s\p{P}\p{S}]/u;
const trigger = /(?:^|[^A-Za-z0-9_.+\-@＠])([@＠])/gu;

/** Exact typed tokens bind only to unique directory keys. Fuzzy suggestions never route. */
export function resolveMessageMentions(
  edit: ComposerEdit,
  agents: readonly AgentDestination[] | undefined,
  includeEnd = true,
): MentionResolution {
  const mentions = [...(edit.mentions ?? [])];
  const unknown: string[] = [],
    ambiguous: string[] = [];
  let unresolved = false;
  const ambiguousTokens: MentionResolution['ambiguousTokens'] = [];
  for (const match of edit.value.matchAll(trigger)) {
    const start = match.index + match[0].length - 1;
    if (mentions.some((token) => start >= token.range.start && start < token.range.end)) continue;
    if (agents === undefined) {
      unresolved = true;
      continue;
    }
    const after = edit.value.slice(start + 1);
    const matches = agents.filter((agent) => {
      if (!after.startsWith(agent.agentName)) return false;
      const next = after[agent.agentName.length];
      return next === undefined ? includeEnd : boundary.test(next);
    });
    const longest = Math.max(0, ...matches.map((agent) => agent.agentName.length));
    const exact = matches.filter((agent) => agent.agentName.length === longest);
    if (exact.length === 1) {
      mentions.push({
        key: { machine: exact[0].machine, agent: exact[0].agent },
        range: { start, end: start + longest + 1 },
      });
    } else if (exact.length) {
      ambiguous.push(exact[0].agentName);
      ambiguousTokens.push({
        name: exact[0].agentName,
        range: { start, end: start + longest + 1 },
      });
    } else {
      const name = after.split(/[\s.,!?;:()[\]{}。！？、，；：「」@＠]/u)[0];
      if (name) unknown.push(name);
    }
  }
  return {
    mentions: mentions.sort((a, b) => a.range.start - b.range.start),
    unknown,
    ambiguous,
    ambiguousTokens,
    unresolved,
  };
}

/** Presentation policy only. Remote and the signed Ask owner retain effect admission. */
export function messageRecipient({
  writable,
  edit,
  destinations,
}: {
  writable: boolean;
  edit: ComposerEdit;
  destinations?: readonly AgentDestination[];
}) {
  const resolution = resolveMessageMentions(edit, destinations);
  const keys = resolution.mentions.filter(
    (mention, index, all) =>
      all.findIndex((other) => recipientId(other.key) === recipientId(mention.key)) === index,
  );
  const agents: AgentDestination[] = [];
  const unavailable: string[] = [];
  for (const token of keys) {
    const matches = destinations?.filter((agent) => recipientId(agent) === recipientId(token.key));
    if (matches?.length === 1 && matches[0].online === 'online') agents.push(matches[0]);
    else unavailable.push(edit.value.slice(token.range.start + 1, token.range.end));
  }
  const allowed =
    writable &&
    !resolution.unresolved &&
    !resolution.ambiguous.length &&
    !unavailable.length &&
    keys.length <= MESSAGE_RECIPIENT_LIMIT;
  return {
    ...resolution,
    agents,
    unavailable,
    tooMany: keys.length > MESSAGE_RECIPIENT_LIMIT,
    allowed,
  };
}
