import { describe, expect, it } from 'vite-plus/test';
import { ChatSeen } from '../src/chat-unread.js';
import type { ReplyNotification } from '../src/reply-notifications.js';

const reply = (
  id: string,
  destination: ReplyNotification['destination'] = 'Chat',
): ReplyNotification => ({ id, destination, firstLine: 'Untrusted display copy', recordKey: id });
function storage() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
  };
}
describe('device-local Chat seen position', () => {
  it('empty is clear; only eligible Chat identities contribute', () => {
    const position = new ChatSeen('space/page/epoch/device', storage());
    expect(position.unread([])).toBe(false);
    expect(
      position.unread([
        reply('answer', 'Answer'),
        reply('proposal', 'Proposal'),
        reply('thread', 'Thread'),
      ]),
    ).toBe(false);
    expect(position.unread([reply('a')])).toBe(true);
    expect(position.read([reply('a'), reply('thread', 'Thread')])).toBe(true);
    expect(position.unread([reply('a')])).toBe(false);
    expect(position.unread([reply('thread')])).toBe(true);
    expect(position.read([reply('a')])).toBe(false);
  });
  it('persists acknowledged identities, not order, timestamp or display edits', () => {
    const local = storage();
    const first = new ChatSeen('space/page/1/device', local);
    first.read([reply('later')]);
    const reloaded = new ChatSeen('space/page/1/device', local);
    expect(reloaded.unread([{ ...reply('later'), firstLine: 'Edited body', revision: '2' }])).toBe(
      false,
    );
    expect(reloaded.unread([reply('earlier'), reply('later')])).toBe(true);
    expect(reloaded.unread([])).toBe(false); // Deleted records cannot keep a cue alive.
    expect(reloaded.unread([reply('later')])).toBe(false); // An unavailable read did not discard position.
    for (const scope of [
      'space/page/2/device',
      'space/page/1/other',
      'space/other/1/device',
      'other/page/1/device',
    ])
      expect(new ChatSeen(scope, local).unread([reply('later')])).toBe(true);
  });
  it('malformed or unavailable storage never falsely acknowledges; this tab retains reads', () => {
    for (const value of ['{}', '[1]', 'null', 'not json']) {
      const position = new ChatSeen('scope', {
        getItem: () => value,
        setItem: () => {
          throw new Error('quota');
        },
      });
      expect(position.unread([reply('a')])).toBe(true);
      position.read([reply('a')]);
      expect(position.unread([reply('a')])).toBe(false);
    }
    const position = new ChatSeen('scope', {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
    });
    expect(position.unread([reply('a')])).toBe(true);
    position.read([reply('a')]);
    expect(position.unread([reply('a')])).toBe(false);
  });
});
