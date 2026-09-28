import { expect, it } from 'vitest';
import { fitNameplateText } from './scene-nameplate.js';

const measure = (text: string) => Array.from(text).length * 6;

it('retains short names and truncates long names within the room instead of shrinking them', () => {
  expect(fitNameplateText('Alice', 36, measure)).toBe('Alice');
  expect(fitNameplateText('Release coordinator', 36, measure)).toBe('Relea…');
  expect(fitNameplateText('Alice', 6, measure)).toBe('…');
  expect(fitNameplateText('Alice', 5, measure)).toBe('');
});

it('preserves code points when truncating a label', () => {
  expect(fitNameplateText('A🤖BC', 18, measure)).toBe('A🤖…');
});
