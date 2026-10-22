import { expect, it } from 'vite-plus/test';
import {
  MESSAGE_STATES,
  clock,
  parseMessageState,
  remainingAt,
  sentenceCase,
  sameMessageState,
  type ObservedMessageState,
} from '../src/message-state.js';

// Core's `messageState` (request-response-v1, Message state) as Remote carries it (#4317): exactly
// five keys, six states, an open reason code, bounded text, and a countdown that is both null or
// both a count. Anything else is dropped whole and the ask shows what it showed before.
const valid = (extra: Record<string, unknown> = {}) => ({
  state: 'batched',
  reason: {
    code: 'held_to_save_usage',
    text: 'batched to save usage, sends when the agent finishes this turn',
  },
  latestMs: 1_700_000_150_000,
  remainingMs: 151_000,
  canSendNow: true,
  ...extra,
});

const without = (value: Record<string, unknown>, key: string) =>
  Object.fromEntries(Object.entries(value).filter(([name]) => name !== key));

it('accepts each of the six states with its reason', () => {
  const rows: [string, string, string][] = [
    ['sent', 'waiting_for_delivery', 'waiting for delivery'],
    [
      'batched',
      'held_to_save_usage',
      'batched to save usage, sends when the agent finishes this turn',
    ],
    ['delivered', 'reached_the_agent', 'delivered'],
    ['working', 'agent_working', 'the agent has started on it'],
    ['unconfirmed', 'handed_over', 'handed to the agent, delivery unconfirmed, do not resend'],
    ['not_sent', 'unreachable', 'not sent, the agent could not be reached'],
  ];
  expect(rows.map(([state]) => state)).toEqual([...MESSAGE_STATES]);
  for (const [state, code, text] of rows) {
    const parsed = parseMessageState({
      state,
      reason: { code, text },
      latestMs: null,
      remainingMs: null,
      canSendNow: state === 'sent',
    });
    expect(parsed).toEqual({
      state,
      reason: { code, text },
      latestMs: null,
      remainingMs: null,
      canSendNow: state === 'sent',
    });
  }
});

it('keeps the countdown a Digest hold carries, and an unknown reason code', () => {
  expect(parseMessageState(valid())).toEqual(valid());
  const future = parseMessageState(
    valid({ reason: { code: 'a_new_reason', text: 'something new' } }),
  );
  expect(future?.reason).toEqual({ code: 'a_new_reason', text: 'something new' });
  // Zero once past, never negative: zero is a count.
  expect(parseMessageState(valid({ remainingMs: 0 }))?.remainingMs).toBe(0);
});

it('drops a value that is not exactly the contract', () => {
  const dropped: [string, unknown][] = [
    ['absent', undefined],
    ['null', null],
    ['an array', []],
    ['a string', 'batched'],
    ['an unknown state', valid({ state: 'queued' })],
    ['a state in the wrong case', valid({ state: 'Batched' })],
    ['a missing key', without(valid(), 'canSendNow')],
    ['an extra key', valid({ extra: 1 })],
    ['a missing reason', without(valid(), 'reason')],
    ['a reason that is a string', valid({ reason: 'delivered' })],
    ['a reason with an extra key', valid({ reason: { code: 'a', text: 'b', more: 1 } })],
    ['a reason without text', valid({ reason: { code: 'held_to_save_usage' } })],
    ['an empty reason text', valid({ reason: { code: 'held_to_save_usage', text: '' } })],
    [
      'a reason text of 513 bytes',
      valid({ reason: { code: 'held_to_save_usage', text: 'x'.repeat(513) } }),
    ],
    [
      'a reason text with a line break',
      valid({ reason: { code: 'held_to_save_usage', text: 'a\nb' } }),
    ],
    ['a reason code in capitals', valid({ reason: { code: 'HELD', text: 'held' } })],
    ['an empty reason code', valid({ reason: { code: '', text: 'held' } })],
    ['a reason code that is a number', valid({ reason: { code: 1, text: 'held' } })],
    ['a deadline without a remainder', valid({ remainingMs: null })],
    ['a remainder without a deadline', valid({ latestMs: null })],
    ['a negative remainder', valid({ remainingMs: -1 })],
    ['a fractional remainder', valid({ remainingMs: 1.5 })],
    ['a remainder past the safe integers', valid({ remainingMs: 2 ** 53 })],
    ['a string remainder', valid({ remainingMs: '151000' })],
    ['an undefined deadline', valid({ latestMs: undefined })],
    ['a non-boolean canSendNow', valid({ canSendNow: 1 })],
    ['a string canSendNow', valid({ canSendNow: 'true' })],
  ];
  for (const [name, value] of dropped) expect(parseMessageState(value), name).toBeUndefined();
});

it('takes a reason text of exactly 512 bytes, counted in UTF-8', () => {
  expect(
    parseMessageState(valid({ reason: { code: 'held_to_save_usage', text: 'x'.repeat(512) } })),
  ).toBeDefined();
  // 171 three-byte characters are 513 bytes.
  expect(
    parseMessageState(valid({ reason: { code: 'held_to_save_usage', text: '火'.repeat(171) } })),
  ).toBeUndefined();
});

it('counts down locally from the read and never goes below zero', () => {
  const observed: ObservedMessageState = { ...parseMessageState(valid())!, receivedAt: 1000 };
  expect(remainingAt(observed, 1000)).toBe(151_000);
  expect(remainingAt(observed, 31_000)).toBe(121_000);
  expect(remainingAt(observed, 5_000_000)).toBe(0);
  // A clock that went backwards does not add time.
  expect(remainingAt(observed, 0)).toBe(151_000);
  expect(remainingAt({ ...observed, latestMs: null, remainingMs: null }, 9000)).toBeNull();
});

it('writes a countdown as m:ss', () => {
  expect(clock(151_000)).toBe('2:31');
  expect(clock(0)).toBe('0:00');
  expect(clock(59_999)).toBe('0:59');
  expect(clock(3_600_000)).toBe('60:00');
  expect(clock(-5)).toBe('0:00');
});

it('capitalizes only the first letter of the text Core gave', () => {
  expect(sentenceCase('waiting for delivery')).toBe('Waiting for delivery');
  expect(sentenceCase("the agent's turn")).toBe("The agent's turn");
  expect(sentenceCase('')).toBe('');
});

it('treats a later read as the same message state unless something visible moved', () => {
  const first: ObservedMessageState = { ...parseMessageState(valid())!, receivedAt: 1000 };
  // Five seconds later the local countdown reads 146 s; Core says 146 s: nothing to publish.
  const same: ObservedMessageState = { ...first, remainingMs: 146_000, receivedAt: 6000 };
  expect(sameMessageState(first, same)).toBe(true);
  // Core's clock disagrees by a few seconds: publish the correction.
  expect(sameMessageState(first, { ...same, remainingMs: 140_000 })).toBe(false);
  expect(sameMessageState(first, { ...same, state: 'delivered' })).toBe(false);
  expect(sameMessageState(first, { ...same, canSendNow: false })).toBe(false);
  expect(sameMessageState(first, { ...same, reason: { code: 'x', text: 'y' } })).toBe(false);
  expect(sameMessageState(first, { ...same, latestMs: 1 })).toBe(false);
  expect(sameMessageState(undefined, first)).toBe(false);
  expect(sameMessageState(first, undefined)).toBe(false);
  expect(sameMessageState(undefined, undefined)).toBe(true);
});
