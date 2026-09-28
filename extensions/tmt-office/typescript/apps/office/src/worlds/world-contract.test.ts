import { describe, expect, it } from 'vitest';
import { validWorldId, validWorldName, WORLD_ID_PATTERN } from './world-contract.js';

describe('world ID contract', () => {
  it.each([
    ['aA09'.repeat(5), true],
    ['a'.repeat(19), false],
    ['a'.repeat(21), false],
    ['a'.repeat(19) + '/', false],
    ['a'.repeat(19) + '_', false],
    ['a'.repeat(19) + 'é', false],
    ['', false],
  ])('validates %j with the same pattern used by the form', (id, valid) => {
    expect(validWorldId(id)).toBe(valid);
    const input = document.createElement('input');
    input.required = true;
    input.pattern = WORLD_ID_PATTERN;
    input.value = id;
    expect(input.checkValidity()).toBe(valid);
  });

  it('rejects trailing line terminators', () => {
    expect(validWorldId('a'.repeat(20) + '\n')).toBe(false);
    expect(validWorldId('a'.repeat(20) + '\r')).toBe(false);
  });
});

describe('world name feedback', () => {
  it.each(['', ' ', '\u00a0', '\u3000', '\ud800', 'line\nbreak', 'x'.repeat(81), '😀'.repeat(81)])(
    'rejects invalid name %j',
    (name) => {
      expect(validWorldName(name)).toBe(false);
    }
  );
  it.each(['My office', 'x'.repeat(80), '😀'.repeat(80)])('accepts valid name %j', (name) => {
    expect(validWorldName(name)).toBe(true);
  });
});
