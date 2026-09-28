import { expect, it } from 'vitest';
import { avatarTopInset } from './avatar-layout.js';

it('anchors names to the first visible portrait row for both admitted index widths', () => {
  const palette = ['#00000000', '#ffffffff'];
  expect(avatarTopInset({ palette, pixels: ['00', '01', '11', '00'] })).toBeCloseTo(2.1);
  expect(
    avatarTopInset({ palette, indexWidth: 2, pixels: ['0000', '0001', '0101', '0000'] })
  ).toBeCloseTo(2.1);
  expect(avatarTopInset({ palette, pixels: ['10', '00'] })).toBe(0);
  expect(avatarTopInset({ palette, pixels: ['00', '00'] })).toBe(0);
});
