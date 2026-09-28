import { Timestamp } from 'firebase/firestore';
import { expect, it } from 'vitest';
import { readAgentSpace } from './firebase-spaces.js';

const id = '00000000-0000-4000-8000-000000000001';
const principal = `office-agent:${id}`;
const value = {
  version: 1,
  ownerUid: 'owner',
  installationId: id,
  identityId: id,
  blockId: id,
  capabilities: ['layout.read'],
  enabled: false,
  createdAt: Timestamp.fromMillis(1000),
  expiresAt: Timestamp.fromMillis(2000),
};
it('keeps revoked and expired records visible without inferring presence or authority', () => {
  expect(readAgentSpace(principal, value, 'owner')).toEqual({
    principalUid: principal,
    installationId: id,
    identityId: id,
    blockId: id,
    capabilities: ['layout.read'],
    enabled: false,
    expiresAtMs: 2000,
  });
  expect(readAgentSpace(principal, { ...value, enabled: true }, 'owner').enabled).toBe(true);
});

it('projects only a disabled grant with an exact valid transfer receipt', () => {
  const receipt = { ...value, replacedByPairingId: 'a'.repeat(64) };
  expect(readAgentSpace(principal, receipt, 'owner').replacedByPairingId).toBe('a'.repeat(64));
  for (const patch of [
    { enabled: true },
    { replacedByPairingId: null },
    { replacedByPairingId: 'x' },
    { extra: 1 },
  ])
    expect(() => readAgentSpace(principal, { ...receipt, ...patch }, 'owner')).toThrow();
});
it('rejects incomplete, expanded, wrong-owner and malformed authority projections', () => {
  for (const key of Object.keys(value)) {
    const incomplete: Record<string, unknown> = { ...value };
    delete incomplete[key];
    expect(() => readAgentSpace(principal, incomplete, 'owner')).toThrow();
  }
  for (const patch of [
    { extra: true },
    { ownerUid: 'other' },
    { version: 2 },
    { enabled: 'true' },
    { blockId: 'home' },
    { identityId: '../identity' },
    { installationId: '' },
    { capabilities: ['layout.write'] },
    { capabilities: ['layout.read', 'board.write'] },
    { createdAt: 1000 },
    { expiresAt: Timestamp.fromMillis(1000) },
    { expiresAt: Timestamp.fromMillis(86_401_001) },
  ])
    expect(() => readAgentSpace(principal, { ...value, ...patch }, 'owner')).toThrow();
  expect(() => readAgentSpace('alice', value, 'owner')).toThrow();
});
