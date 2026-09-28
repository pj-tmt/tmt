import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { PixelCanvas } from './pixel-canvas.js';
import { newPixelDraft } from './pixel-draft.js';

beforeEach(() => {
  class Pointer extends MouseEvent {
    pointerId: number;
    constructor(type: string, input: PointerEventInit = {}) {
      super(type, input);
      this.pointerId = input.pointerId ?? 1;
    }
  }
  vi.stubGlobal('PointerEvent', Pointer);
});
afterEach(() => vi.unstubAllGlobals());

function setup() {
  const commit = vi.fn();
  const strokeChanged = vi.fn();
  const draft = newPixelDraft();
  const props = { draft, color: 3, disabled: false, commit, strokeChanged };
  const view = render(<PixelCanvas {...props} />);
  const canvas = screen.getByRole('group', { name: 'Pixel drawing canvas' });
  Object.assign(canvas, {
    setPointerCapture: vi.fn(),
    releasePointerCapture: vi.fn(),
    getBoundingClientRect: () => ({ left: 12.25, top: 20.75, width: 253.5, height: 253.5 }),
  });
  const point = (x: number, y: number) => ({
    pointerId: 4,
    button: 0,
    clientX: 12.25 + ((x + 0.5) / 16) * 253.5,
    clientY: 20.75 + ((y + 0.5) / 16) * 253.5,
  });
  return { ...view, canvas, props, point, commit, strokeChanged };
}

it('commits one captured stroke and erases the same cell with fractional layout coordinates', () => {
  const { canvas, props, point, commit, rerender, strokeChanged } = setup();
  fireEvent.pointerDown(canvas, point(8, 8));
  expect(commit).not.toHaveBeenCalled();
  fireEvent.pointerUp(canvas, point(8, 8));
  expect(commit).toHaveBeenCalledTimes(1);
  const painted = commit.mock.calls[0]![0];
  expect(painted.pixels[8]).toBe(`${'00'.repeat(8)}03${'00'.repeat(7)}`);
  expect(strokeChanged.mock.calls).toEqual([[true], [false]]);
  rerender(<PixelCanvas {...props} draft={painted} color={0} />);
  fireEvent.pointerDown(canvas, point(8, 8));
  fireEvent.pointerUp(canvas, point(8, 8));
  expect(commit).toHaveBeenCalledTimes(2);
  expect(commit.mock.calls[1]![0]).toEqual(props.draft);
});

it('takes keyboard focus without scrolling the drawing surface during a pointer stroke', () => {
  const { canvas, point, commit } = setup();
  const focus = vi.spyOn(canvas, 'focus');
  fireEvent.pointerDown(canvas, point(3, 4));
  expect(focus).toHaveBeenCalledWith({ preventScroll: true });
  fireEvent.pointerMove(canvas, point(12, 4));
  fireEvent.pointerUp(canvas, point(12, 4));
  expect(commit).toHaveBeenCalledTimes(1);
  expect(commit.mock.calls[0]![0].pixels[4]).toBe(
    `${'00'.repeat(3)}${'03'.repeat(10)}${'00'.repeat(3)}`
  );
});

it.each(['pointerCancel', 'lostPointerCapture', 'blur'] as const)(
  'discards a pending stroke on %s and accepts the next independent stroke',
  (event) => {
    const { canvas, point, commit, strokeChanged } = setup();
    fireEvent.pointerDown(canvas, point(2, 3));
    fireEvent[event](canvas);
    fireEvent.pointerUp(canvas, point(2, 3));
    expect(commit).not.toHaveBeenCalled();
    expect(strokeChanged.mock.calls).toEqual([[true], [false]]);
    fireEvent.pointerDown(canvas, point(8, 8));
    fireEvent.pointerUp(canvas, point(8, 8));
    expect(commit).toHaveBeenCalledTimes(1);
    expect(commit.mock.calls[0]![0].pixels[3]).toBe('00'.repeat(16));
    expect(commit.mock.calls[0]![0].pixels[8].slice(16, 18)).toBe('03');
  }
);
