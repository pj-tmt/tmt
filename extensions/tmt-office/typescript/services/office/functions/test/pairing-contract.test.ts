import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import {
  parseApproval,
  parseClaim,
  parseOwnerApproval,
  parseRenewal,
  parseRevocation,
} from '../src/pairing-contract.js';
import { serviceEnvironment } from '../src/firebase-environment.js';

const approval = {
  version: 1,
  pairingId: 'a'.repeat(64),
  worldId: 'A'.repeat(20),
  installationId: '00000000-0000-4000-8000-000000000001',
  identityId: '00000000-0000-4000-8000-000000000002',
  identityLabel: 'Alice',
  installationLabel: 'Workstation',
  capabilities: ['layout.read', 'layout.write'],
};

describe('pairing input contract', () => {
  it('accepts only the explicit versioned binding and canonical layout capabilities', () => {
    expect(parseApproval(approval)).toEqual(approval);
    expect(parseApproval({ ...approval, capabilities: ['layout.read'] }).capabilities).toEqual([
      'layout.read',
    ]);
    expect(parseRevocation({ version: 1, pairingId: approval.pairingId })).toEqual({
      kind: 'pairing',
      pairingId: approval.pairingId,
    });
    expect(parseRevocation({ version: 1, publicApproval: approval })).toEqual({
      kind: 'ownerApproval',
      request: approval,
    });
    expect(parseRenewal({ version: 1, pairingId: approval.pairingId, grantExpiresAt: 1 })).toEqual({
      version: 1,
      pairingId: approval.pairingId,
      grantExpiresAt: 1,
    });
    expect(() =>
      parseRenewal({ version: 1, pairingId: approval.pairingId, grantExpiresAt: 1, extra: true })
    ).toThrow('INVALID_ARGUMENT');
  });
  it('keeps the owner replacement selector outside the native request contract', () => {
    const principal = 'office-agent:00000000-0000-4000-8000-000000000099';
    expect(parseOwnerApproval({ ...approval, replacesPrincipalUid: principal })).toEqual({
      request: approval,
      replacesPrincipalUid: principal,
    });
    expect(() => parseApproval({ ...approval, replacesPrincipalUid: principal })).toThrow(
      'INVALID_ARGUMENT'
    );
    for (const replacesPrincipalUid of [
      '00000000-0000-4000-8000-000000000099',
      'office-agent:BAD',
      null,
      undefined,
    ])
      expect(() => parseOwnerApproval({ ...approval, replacesPrincipalUid })).toThrow(
        'INVALID_ARGUMENT'
      );
  });
  it.each([
    { ownerUid: 'self-grant' },
    { version: 2 },
    { worldId: '../world' },
    { identityId: 'alice' },
    { installationId: 'pane-1' },
    { pairingId: 'short' },
    { identityLabel: '' },
    { identityLabel: ' '.repeat(5) },
    { identityLabel: 'x'.repeat(81) },
    { identityLabel: 'line\nbreak' },
    { identityLabel: '\ud800' },
    { capabilities: ['layout.write'] },
    { capabilities: ['layout.read', 'board.write'] },
    { capabilities: ['layout.read', 'layout.read'] },
  ])('rejects altered binding %j', (change) => {
    expect(() => parseApproval({ ...approval, ...change })).toThrow('INVALID_ARGUMENT');
  });
  it.each([
    { version: 2, publicApproval: approval },
    { version: 1, publicApproval: { ...approval, capabilities: ['layout.write'] } },
    { version: 1, publicApproval: approval, pairingId: approval.pairingId },
    { version: 1, publicApproval: approval, secret: 'invalid' },
  ])('rejects altered public approval revocations %j', (input) => {
    expect(() => parseRevocation(input)).toThrow('INVALID_ARGUMENT');
  });
  it('rejects every omitted field, null, arrays and inherited fields', () => {
    for (const field of Object.keys(approval)) {
      const incomplete: Record<string, unknown> = { ...approval };
      delete incomplete[field];
      expect(() => parseApproval(incomplete)).toThrow('INVALID_ARGUMENT');
    }
    for (const value of [null, [], Object.create(approval)])
      expect(() => parseApproval(value)).toThrow('INVALID_ARGUMENT');
  });
  it.each([0, -1, 1.5, Number.MAX_SAFE_INTEGER, 8_640_000_000_000_001])(
    'rejects an invalid renewal expiry %s',
    (grantExpiresAt) => {
      expect(() =>
        parseRenewal({ version: 1, pairingId: approval.pairingId, grantExpiresAt })
      ).toThrow('INVALID_ARGUMENT');
    }
  );
  it('hashes original proof bytes and rejects noncanonical or oversized secrets', () => {
    const bytes = Buffer.alloc(32, 1);
    const secret = bytes.toString('base64url');
    expect(parseClaim({ version: 1, secret })).toBe(
      createHash('sha256').update(bytes).digest('hex')
    );
    for (const value of [secret + '=', secret.slice(1), 'x'.repeat(44), secret.slice(0, -1) + 'R'])
      expect(() => parseClaim({ version: 1, secret: value })).toThrow('INVALID_ARGUMENT');
    expect(() => parseClaim({ version: 1, secret, ownerUid: 'extra' })).toThrow('INVALID_ARGUMENT');
  });
});

describe('service activation boundary', () => {
  it('defaults off outside emulators and requires an exact explicit HTTPS origin', () => {
    expect(serviceEnvironment({})).toEqual({ enabled: false });
    expect(
      serviceEnvironment({
        TMT_OFFICE_PAIRING_ENABLED: 'true',
        TMT_OFFICE_ORIGIN: 'https://office.example',
      })
    ).toEqual({ enabled: true, origin: 'https://office.example' });
    for (const origin of [
      '',
      'http://office.example',
      'https://office.example/path',
      'https://user@office.example',
    ])
      expect(() =>
        serviceEnvironment({ TMT_OFFICE_PAIRING_ENABLED: 'true', TMT_OFFICE_ORIGIN: origin })
      ).toThrow();
  });
  it('refuses incomplete emulation, real projects and remote emulator endpoints', () => {
    const env = {
      FUNCTIONS_EMULATOR: 'true',
      GCLOUD_PROJECT: 'demo-tmt-office',
      FIREBASE_AUTH_EMULATOR_HOST: '127.0.0.1:9099',
      FIRESTORE_EMULATOR_HOST: '127.0.0.1:8080',
    };
    expect(serviceEnvironment(env).enabled).toBe(true);
    for (const change of [
      { FUNCTIONS_EMULATOR: 'false' },
      { GCLOUD_PROJECT: 'real-project' },
      { FIRESTORE_EMULATOR_HOST: 'remote.example:8080' },
      { FIREBASE_AUTH_EMULATOR_HOST: undefined },
    ])
      expect(() => serviceEnvironment({ ...env, ...change })).toThrow('isolated demo');
  });
});
