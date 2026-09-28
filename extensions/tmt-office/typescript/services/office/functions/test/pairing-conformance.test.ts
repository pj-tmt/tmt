import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { parseApproval } from '../src/pairing-contract.js';

interface PairingVectors {
  valid: unknown[];
  invalid: unknown[];
  invalidJson: string[];
}

const vectors = JSON.parse(
  readFileSync(
    new URL('../../../../../../../contracts/office/pairing-examples.json', import.meta.url),
    'utf8'
  )
) as PairingVectors;

describe('pairing request conformance', () => {
  it.each(vectors.invalidJson)('rejects malformed Unicode in raw JSON %#', (input) => {
    expect(() => parseApproval(JSON.parse(input))).toThrow('INVALID_ARGUMENT');
  });
  it('keeps positive and negative vectors instead of passing an empty corpus', () => {
    expect(vectors.valid.length).toBeGreaterThan(0);
    expect(vectors.invalid.length).toBeGreaterThan(0);
  });
  it.each(vectors.valid)('accepts the literal valid request %#', (request) => {
    expect(parseApproval(request)).toEqual(request);
  });

  it.each(vectors.invalid)('rejects the literal invalid request %#', (request) => {
    expect(() => parseApproval(request)).toThrow('INVALID_ARGUMENT');
  });
});
