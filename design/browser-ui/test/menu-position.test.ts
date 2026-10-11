import { expect, it } from 'vite-plus/test';
import { browserMenuPosition } from '../src/menu-position.js';

const viewport = { left: 0, top: 0, width: 390, height: 500 };
it('places below when the content fits', () => {
  expect(
    browserMenuPosition({ left: 300, right: 380, top: 50, bottom: 80 }, 160, 100, viewport),
  ).toEqual({ left: 220, top: 80, maxWidth: 374, maxHeight: 412 });
});
it('flips above a bottom anchor', () => {
  expect(
    browserMenuPosition({ left: 300, right: 380, top: 450, bottom: 480 }, 160, 100, viewport),
  ).toEqual({ left: 220, top: 350, maxWidth: 374, maxHeight: 442 });
});
it('constrains the larger side when neither side fits, including a visual viewport offset', () => {
  const result = browserMenuPosition({ left: 0, right: 30, top: 220, bottom: 250 }, 700, 700, {
    left: 10,
    top: 20,
    width: 390,
    height: 400,
  });
  expect(result).toEqual({ left: 18, top: 28, maxWidth: 374, maxHeight: 192 });
});
