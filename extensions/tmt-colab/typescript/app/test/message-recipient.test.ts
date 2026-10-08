import { expect, it } from 'vite-plus/test';
import { messageRecipient, resolveMessageMentions } from '../src/message-recipient.js';
import type { AgentDestination } from '../src/live-ask.js';
import type { ComposerEdit } from '../src/components/message-composer-edit.js';
import { destination, id } from './ask-fixtures.js';
const a = { ...destination(), agentName: 'alpha' };
const b = { ...a, agent: id(9), agentName: 'beta' };
const decide = (edit: ComposerEdit, destinations: AgentDestination[] = [a, b]) =>
  messageRecipient({ writable: true, edit, destinations });

it('posts mention-free comments without directory admission, including an empty or failed directory', () => {
  for (const destinations of [undefined, [], [a]]) {
    const result = messageRecipient({
      writable: true,
      edit: { value: 'email@alpha stays text' },
      destinations,
    });
    expect(result.allowed).toBe(true);
    expect(result.agents).toEqual([]);
  }
  expect(messageRecipient({ writable: false, edit: { value: 'note' } }).allowed).toBe(false);
});
it('exact typed names bind on space, punctuation or explicit end, preserving UTF16 ranges', () => {
  for (const tail of [' ', ',', '。', '']) {
    const result = resolveMessageMentions({ value: `😀 @alpha${tail}` }, [a]);
    expect(result.mentions).toEqual([
      { key: { machine: a.machine, agent: a.agent }, range: { start: 3, end: 9 } },
    ]);
  }
  expect(resolveMessageMentions({ value: '@alpha' }, [a], false).mentions).toEqual([]);
  expect(decide({ value: '請＠alpha explain' }).agents).toEqual([a]);
});
it('fans out to all distinct tokens, deduping repeated mentions by UUID pair', () => {
  expect(decide({ value: '@alpha @beta @alpha Explain.' }).agents).toEqual([a, b]);
});
it('keeps unknown names as text and ambiguous exact names unbound', () => {
  const unknown = decide({ value: '@missing note' });
  expect(unknown).toMatchObject({ allowed: true, unknown: ['missing'], agents: [] });
  const ambiguous = decide({ value: '@alpha note' }, [a, { ...b, agentName: 'alpha' }]);
  expect(ambiguous).toMatchObject({ allowed: false, ambiguous: ['alpha'], mentions: [] });
});
it('preserves a bound UUID across renames and never substitutes a same-label agent or machine', () => {
  const edit = {
    value: '@alpha note',
    mentions: [{ key: { machine: a.machine, agent: a.agent }, range: { start: 0, end: 6 } }],
  };
  expect(decide(edit, [{ ...a, agentName: 'renamed' }]).agents[0].agent).toBe(a.agent);
  for (const replacement of [{ ...a, agent: id(8) }, { ...a, machine: id(8) }, a]) {
    const candidates = replacement === a ? [a, a] : [replacement];
    expect(decide(edit, candidates)).toMatchObject({ allowed: false, unavailable: ['alpha'] });
  }
});
it('fences mentions against loading/failed directory but permits offline and unknown agent presence on an online machine', () => {
  expect(messageRecipient({ writable: true, edit: { value: '@alpha note' } }).allowed).toBe(false);
  for (const presence of ['offline', 'unknown'] as const) {
    expect(decide({ value: '@alpha note' }, [{ ...a, presence }]).allowed).toBe(true);
  }
  expect(decide({ value: '@alpha note' }, [{ ...a, online: 'offline' }]).allowed).toBe(false);
});
it('allows eight distinct recipients and refuses nine before any effect', () => {
  const agents = Array.from({ length: 9 }, (_, i) => ({
    ...a,
    agent: id(i + 20),
    agentName: `agent-${i}`,
  }));
  for (const count of [8, 9]) {
    const result = decide(
      {
        value: agents
          .slice(0, count)
          .map((agent) => `@${agent.agentName}`)
          .join(' '),
      },
      agents,
    );
    expect(result.allowed).toBe(count === 8);
    expect(result.tooMany).toBe(count === 9);
  }
});
it('matches complete multiword names before shorter prefixes without fuzzy routing', () => {
  const full = { ...b, agentName: 'alpha beta' };
  expect(decide({ value: '@alpha beta, note' }, [a, full]).agents).toEqual([full]);
  expect(decide({ value: '@alp note' }).agents).toEqual([]);
});
