import { expect, it } from 'vite-plus/test';
import { messageRecipient, type RecipientKey } from '../src/message-recipient.js';
import type { ComposerEdit } from '../src/components/message-composer-edit.js';
import { destination, id } from './ask-fixtures.js';

const agent = destination();
const key: RecipientKey = { machine: agent.machine, agent: agent.agent };
const another = { ...agent, agent: id(9), agentName: 'Other agent' };
const base = { writable: true, intent: 'agent' as const, destinations: [agent, another] };

it('allows an admitted plain comment despite failed or empty agent discovery', () => {
  for (const destinations of [undefined, [], [agent]]) {
    expect(messageRecipient({ writable: true, intent: 'comment', destinations })).toEqual({
      kind: 'comment',
    });
  }
});

it('keeps content-write admission authoritative for comment and agent intent', () => {
  for (const intent of ['comment', 'agent'] as const)
    expect(messageRecipient({ ...base, intent, writable: false })).toEqual({ kind: 'blocked' });
});

it('routes an explicitly selected admitted identity without requiring a mention', () => {
  const edit: ComposerEdit = {
    value: 'Please explain.\n\nKeep this trailing line.\n',
    recipient: key,
  };
  expect(messageRecipient({ ...base, selected: edit.recipient })).toEqual({
    kind: 'agent',
    destination: agent,
  });
});

it('continues a uniquely bound reply by UUID across renames and repeated turns', () => {
  const renamed = { ...agent, agentName: 'Renamed agent' };
  expect(
    messageRecipient({ ...base, destinations: [renamed, another], replyRecipients: [key, key] }),
  ).toEqual({ kind: 'agent', destination: renamed });
});

it('never substitutes a same-label different UUID or another machine for a bound recipient', () => {
  for (const impostor of [
    { ...agent, agent: id(8) },
    { ...agent, machine: id(8) },
  ])
    expect(messageRecipient({ ...base, destinations: [impostor], replyRecipients: [key] })).toEqual(
      {
        kind: 'unavailable',
      },
    );
});

it('requires an explicit choice for a conversation with multiple bound recipients', () => {
  const replyRecipients = [key, { machine: another.machine, agent: another.agent }];
  expect(messageRecipient({ ...base, replyRecipients })).toEqual({ kind: 'choose' });
  expect(messageRecipient({ ...base, replyRecipients, selected: key })).toEqual({
    kind: 'agent',
    destination: agent,
  });
});

it('uses admitted last-replier, publisher or sole default without a text prefix', () => {
  expect(messageRecipient({ ...base, lastReplier: key })).toEqual({
    kind: 'agent',
    destination: agent,
  });
  expect(messageRecipient({ ...base, publisher: key })).toEqual({
    kind: 'agent',
    destination: agent,
  });
  expect(messageRecipient({ ...base, destinations: [agent] })).toEqual({
    kind: 'agent',
    destination: agent,
  });
});

it('uses publisher identity despite duplicate labels and requires a choice without an identity', () => {
  const destinations = [agent, { ...another, agentName: agent.agentName }];
  expect(messageRecipient({ ...base, destinations, publisher: key })).toEqual({
    kind: 'agent',
    destination: agent,
  });
  expect(messageRecipient({ ...base, destinations })).toEqual({ kind: 'choose' });
});

it('does not silently retarget an unavailable explicit or prior choice', () => {
  for (const selection of [{ selected: key }, { replyRecipients: [key] }, { lastReplier: key }])
    expect(messageRecipient({ ...base, destinations: [another], ...selection })).toEqual({
      kind: 'unavailable',
    });
});

it('requires current discovered admission for agent intent and rejects unavailable candidates', () => {
  expect(messageRecipient({ ...base, destinations: undefined, selected: key })).toEqual({
    kind: 'unavailable',
  });
  for (const unavailable of [
    { ...agent, online: 'offline' as const },
    { ...agent, online: 'unknown' as const },
    { ...agent, presence: 'offline' as const },
  ])
    expect(messageRecipient({ ...base, destinations: [unavailable], selected: key })).toEqual({
      kind: 'unavailable',
    });
  expect(messageRecipient({ ...base, destinations: [] })).toEqual({ kind: 'choose' });
});

it('refuses duplicate candidate identity instead of treating it as unique', () => {
  expect(messageRecipient({ ...base, destinations: [agent, { ...agent }], selected: key })).toEqual(
    {
      kind: 'unavailable',
    },
  );
});
