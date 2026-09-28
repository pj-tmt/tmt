import { describe, expect, it } from 'vitest';
import { Buffer } from 'node:buffer';
import vectors from '../../../../../../../contracts/office/pairing-examples.json' with { type: 'json' };
import {
  parseApprovedPairing,
  parsePairingFragment,
  parseRevokedPairing,
  type PairingRequest,
} from './pairing-contract.js';

type PairingVectors = { valid: PairingRequest[]; invalid: unknown[]; invalidJson: string[] };
const pairingVectors = vectors as PairingVectors;
const BASE64URL = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';

function encodeBytes(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString('base64url');
}

function encodeJson(value: unknown): string {
  return encodeBytes(new TextEncoder().encode(JSON.stringify(value)));
}

function fragment(value: unknown, hash = true): string {
  return `${hash ? '#' : ''}tmt-pair=${encodeJson(value)}`;
}

function noncanonical(encoded: string): string {
  const last = encoded.length - 1;
  const digit = BASE64URL.indexOf(encoded[last]);
  const mask = encoded.length % 4 === 2 ? 0b110000 : 0b111100;
  const replacement = (digit & mask) | 1;
  return `${encoded.slice(0, last)}${BASE64URL[replacement]}`;
}

describe('pairing request vectors', () => {
  it.each(pairingVectors.invalidJson)('rejects malformed Unicode in raw JSON %#', (input) => {
    const encoded = Buffer.from(input, 'utf8').toString('base64url');
    expect(() =>
      parsePairingFragment(`#tmt-pair=${encoded}`, pairingVectors.valid[0].worldId)
    ).toThrow('Invalid pairing input.');
  });
  it('keeps a nonempty positive and negative conformance corpus', () => {
    expect(pairingVectors.valid.length).toBeGreaterThan(0);
    expect(pairingVectors.invalid.length).toBeGreaterThan(0);
  });
  it.each(pairingVectors.valid)('accepts the literal valid request %#', (request) => {
    expect(parsePairingFragment(fragment(request), request.worldId)).toEqual(request);
    expect(parsePairingFragment(fragment(request, false), request.worldId)).toEqual(request);
  });

  it.each(pairingVectors.invalid)('rejects the literal invalid request %#', (request) => {
    expect(() => parsePairingFragment(fragment(request), pairingVectors.valid[0].worldId)).toThrow(
      'Invalid pairing input.'
    );
  });

  it('rejects route mismatch, query-like fragments, oversized encodings and invalid UTF-8', () => {
    const request = pairingVectors.valid[0];
    const json = JSON.stringify(request);
    const padded = json.length % 3 === 0 ? `${json} ` : json;
    const encoded = Buffer.from(padded).toString('base64url');
    expect(() => parsePairingFragment(fragment(request), 'B'.repeat(20))).toThrow();
    expect(() => parsePairingFragment(`${fragment(request)}&extra`, request.worldId)).toThrow();
    expect(() => parsePairingFragment(`#tmt-pair=${'A'.repeat(4097)}`, request.worldId)).toThrow();
    const altered = noncanonical(encoded);
    expect(Buffer.from(altered, 'base64url')).toEqual(Buffer.from(encoded, 'base64url'));
    expect(() => parsePairingFragment(`#tmt-pair=${altered}`, request.worldId)).toThrow();
    expect(() =>
      parsePairingFragment(
        `#tmt-pair=${encodeBytes(new Uint8Array([123, 34, 0xc3, 0x28, 125]))}`,
        request.worldId
      )
    ).toThrow();
    const atLimit = ' '.repeat(2048 - Buffer.byteLength(json)) + json;
    expect(
      parsePairingFragment(
        `#tmt-pair=${Buffer.from(atLimit).toString('base64url')}`,
        request.worldId
      )
    ).toEqual(request);
    expect(() =>
      parsePairingFragment(
        `#tmt-pair=${Buffer.from(` ${atLimit}`).toString('base64url')}`,
        request.worldId
      )
    ).toThrow();
  });
});

describe('approved and revoked response decoding', () => {
  const request = pairingVectors.valid[0];
  const approved = {
    ...request,
    principalUid: 'office-agent:00000000-0000-4000-8000-000000000003',
    blockId: '00000000-0000-4000-8000-000000000004',
    expiresAt: 1_800_000_000_000,
  };

  it('accepts an exact approved response and revocation response', () => {
    expect(parseApprovedPairing(approved, request)).toEqual(approved);
    expect(
      parseRevokedPairing(
        { version: 1, pairingId: request.pairingId, revoked: true },
        request.pairingId
      )
    ).toBeUndefined();
  });

  it.each([
    { customToken: 'never accepted' },
    { ownerUid: 'never accepted' },
    { principalUid: 'agent' },
    { blockId: 'block' },
    { expiresAt: 0 },
    { expiresAt: 1.5 },
    { expiresAt: Number.MAX_SAFE_INTEGER + 1 },
    { expiresAt: 8_640_000_000_000_001 },
    { identityId: '00000000-0000-4000-8000-000000000099' },
  ])('rejects altered approved response %#', (change) => {
    expect(() => parseApprovedPairing({ ...approved, ...change }, request)).toThrow(
      'Invalid pairing input.'
    );
  });

  it.each([
    { version: 2, pairingId: request.pairingId, revoked: true },
    { version: 1, pairingId: '0'.repeat(64), revoked: true },
    { version: 1, pairingId: request.pairingId, revoked: false },
    { version: 1, pairingId: request.pairingId, revoked: true, extra: true },
  ])('rejects altered revoke response %#', (value) => {
    expect(() => parseRevokedPairing(value, request.pairingId)).toThrow('Invalid pairing input.');
  });
});
