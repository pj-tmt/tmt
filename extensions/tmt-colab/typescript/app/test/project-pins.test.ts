import { describe, expect, it } from 'vite-plus/test';
import { PROJECT_PINS } from '../src/bootstrap.js';
import { validPins } from '../src/project-pins.js';

describe('the pinned projects of the owner', () => {
  it('accepts a list the owner can store and refuses repeats, bad names and a 21st pin', () => {
    expect(validPins([])).toBe(true);
    expect(validPins(['a', 'q4 plan', '漢'.repeat(32)])).toBe(true);
    expect(validPins(['a', 'a'])).toBe(false);
    expect(validPins(['a', ' lead'])).toBe(false);
    expect(validPins(['a', ''])).toBe(false);
    const full = Array.from({ length: PROJECT_PINS }, (_, n) => `p${n}`);
    expect(PROJECT_PINS).toBe(20);
    expect(validPins(full)).toBe(true);
    expect(validPins([...full, 'one more'])).toBe(false);
  });
});
