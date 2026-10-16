import { expect, it } from 'vite-plus/test';
import {
  messageSendRefusal,
  messageSendSaved,
  messageSendCopy,
  messageSendFailureReference,
  type MessageSendGate,
} from '../src/message-send-refusal.js';
import { OwnAppendRefused, OwnRecordConflict } from '../src/own-publication.js';
import { SessionEndedError, SessionEvictedError } from '../src/ask-remote.js';

const ready: MessageSendGate = {
  sending: false,
  recorded: false,
  archived: false,
  blocked: false,
  discussion: true,
  allowed: true,
  needsBinding: true,
  binding: true,
  value: 'Kept draft',
};
for (const [state, reference] of [
  [{ sending: true }, 'S1'],
  [{ recorded: true }, 'S2'],
  [{ archived: true }, 'S3'],
  [{ blocked: true }, 'S4'],
  [{ discussion: false }, 'S5'],
  [{ allowed: false }, 'S6'],
  [{ binding: false }, 'S7'],
  [{ value: ' \n ' }, 'S8'],
] as const)
  it(`refuses ${reference} before recording without changing the draft`, () => {
    const input = { ...ready, ...state };
    const before = structuredClone(input);
    expect(messageSendRefusal(input)).toBe(reference);
    expect(input).toEqual(before);
  });
it('permits admitted comments without an Ask binding and gives an active send priority', () => {
  expect(messageSendRefusal(ready)).toBeUndefined();
  expect(messageSendRefusal({ ...ready, needsBinding: false, binding: false })).toBeUndefined();
  expect(messageSendRefusal({ ...ready, sending: true, blocked: true })).toBe('S1');
});
for (const [stage, error, reference] of [
  ['admission', new Error('private diagnostic'), 'S20'],
  ['capture', new Error('private diagnostic'), 'S9'],
  ['attachments', new Error('private diagnostic'), 'S11'],
  ['record', new Error('private diagnostic'), 'S12'],
  ['dispatch', new Error('private diagnostic'), 'S13'],
  ['record', new OwnRecordConflict(), 'S15'],
  ['record', new OwnAppendRefused('CONFLICT'), 'S16'],
  ['record', new SessionEndedError('REMOTE_SESSION_ENDED'), 'S17'],
  ['record', new SessionEvictedError(2), 'S18'],
  ['record', new SessionEndedError('REMOTE_SEQUENCE_UNAVAILABLE'), 'S19'],
] as const)
  it(`maps the handled ${reference} branch to a fixed reference`, () => {
    expect(messageSendFailureReference(stage, error)).toBe(reference);
  });
it('never copies a claimed code, message, payload or identifier into the reference', () => {
  expect(
    messageSendFailureReference('record', {
      code: 'S999',
      message: 'private value',
      id: 'private id',
    }),
  ).toBe('S12');
});

for (const reference of ['S2', 'S13'] as const)
  it(`${reference} announces a saved state without error wording or retry guidance`, () => {
    expect(messageSendSaved(reference)).toBe(true);
    expect(messageSendCopy(reference)).toBe(
      'Saved, but delivery to the agent is unconfirmed. Check the thread before sending again.',
    );
    expect(messageSendCopy(reference)).not.toMatch(/✗|Not sent|try again/);
  });
it('keeps pre-publication failures distinct and empty submissions silent', () => {
  expect(messageSendSaved('S12')).toBe(false);
  expect(messageSendCopy('S12')).toBe('Not sent. Your draft is kept; try again.');
  expect(messageSendCopy('S8')).toBe('');
});
