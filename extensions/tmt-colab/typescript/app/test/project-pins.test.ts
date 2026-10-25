import { describe, expect, it } from 'vite-plus/test';
import { PROJECT_PINS } from '../src/bootstrap.js';
import { menuProjects, movedPin, validPins, withPin } from '../src/project-pins.js';

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

  it('lists the pins the space still has, in the order of the owner, or every project when a source has none', () => {
    const projects = ['a', 'b', 'c'];
    expect(menuProjects(projects, undefined)).toEqual({ names: projects, automatic: false });
    expect(menuProjects(projects, { names: ['c', 'a'], automatic: false })).toEqual({
      names: ['c', 'a'],
      automatic: false,
    });
    // A name the space no longer has drops out; an empty chosen list lists none.
    expect(menuProjects(projects, { names: ['gone', 'b'], automatic: true })).toEqual({
      names: ['b'],
      automatic: true,
    });
    expect(menuProjects(projects, { names: [], automatic: false }).names).toEqual([]);
  });

  it('pins last, keeps a pinned project where it is, and unpins without moving the rest', () => {
    expect(withPin(['a', 'b'], 'c', true)).toEqual(['a', 'b', 'c']);
    expect(withPin(['a', 'b'], 'a', true)).toEqual(['a', 'b']);
    expect(withPin(['a', 'b', 'c'], 'b', false)).toEqual(['a', 'c']);
    expect(withPin(['a'], 'x', false)).toEqual(['a']);
  });

  it('moves a pin one place and stays put at either end', () => {
    expect(movedPin(['a', 'b', 'c'], 'b', -1)).toEqual(['b', 'a', 'c']);
    expect(movedPin(['a', 'b', 'c'], 'b', 1)).toEqual(['a', 'c', 'b']);
    expect(movedPin(['a', 'b', 'c'], 'a', -1)).toEqual(['a', 'b', 'c']);
    expect(movedPin(['a', 'b', 'c'], 'c', 1)).toEqual(['a', 'b', 'c']);
    expect(movedPin(['a', 'b'], 'missing', 1)).toEqual(['a', 'b']);
  });
});
