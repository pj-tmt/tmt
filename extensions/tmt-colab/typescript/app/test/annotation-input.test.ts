import { expect, it } from 'vite-plus/test';
import { mentionedDestination, publishingDestination } from '../src/annotation-input.js';
import { destination, id } from './ask-fixtures.js';
import { validateProjection } from '../src/fold-protocol.js';

it('defaults only to a unique reachable publishing name and keeps ambiguous names without a default', () => {
  const target = { ...destination(), agentName: 'publisher', presence: 'active' as const };
  expect(publishingDestination([target], 'publisher')).toBe(target);
  for (const agents of [
    [{ ...target, online: 'offline' as const }],
    [{ ...target, presence: 'offline' as const }],
    [target, { ...target, agent: id(9) }],
  ])
    expect(publishingDestination(agents, 'publisher')).toBeUndefined();
  expect(publishingDestination([target], undefined)).toBeUndefined();
  expect(mentionedDestination('@publisher Explain this', [target])).toBe(target);
  expect(mentionedDestination('@publisher-other Explain this', [target])).toBeUndefined();
  expect(
    mentionedDestination('@publisher Explain this', [target, { ...target, agent: id(9) }]),
  ).toBeUndefined();
});
it('accepts bounded publisher metadata and rejects type, control and Unicode byte overflows', () => {
  for (const publisherAgent of ['publisher', 'é'.repeat(64)])
    expect(() => validateProjection({ source: '', title: '', publisherAgent })).not.toThrow();
  for (const publisherAgent of [null, 1, '', 'é'.repeat(65), 'line\nbreak'])
    expect(() => validateProjection({ source: '', title: '', publisherAgent })).toThrow();
});
