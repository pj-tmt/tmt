import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { BUILTIN_DIGEST, BUILTIN_PACK } from '../props/prop-contract.js';
import { useCatalogDrag } from './use-catalog-drag.js';
import type { WorldObject } from './world-contract.js';

const pack = { digest: BUILTIN_DIGEST, pack: BUILTIN_PACK };
const commit = vi.fn();
const choose = vi.fn();
const clear = vi.fn();
const preview = vi.fn((object: WorldObject) => ({
  object,
  problem: undefined as string | undefined,
}));
const release = vi.fn();
let captured = false;

function Harness({ disabled = false }) {
  const drag = useCatalogDrag({ disabled, commit });
  drag.placement.current = { preview, clear };
  return (
    <>
      <button
        onPointerDown={(event) => drag.handlers.start(event, pack, 'desk')}
        onClick={(event) => {
          if (!drag.handlers.consumeClick(event.detail)) choose();
        }}
      >
        Desk
      </button>
      <output>{drag.feedback?.message}</output>
    </>
  );
}

function pointer(target: EventTarget, type: string, x: number, pointerType = 'mouse') {
  const event = new MouseEvent(type, { bubbles: true, clientX: x, clientY: 10, button: 0 });
  Object.defineProperties(event, { pointerId: { value: 1 }, pointerType: { value: pointerType } });
  act(() => {
    target.dispatchEvent(event);
  });
}
beforeEach(() => {
  vi.clearAllMocks();
  captured = false;
  preview.mockImplementation((object) => ({ object, problem: undefined }));
  Object.defineProperties(HTMLElement.prototype, {
    setPointerCapture: {
      configurable: true,
      value: () => {
        captured = true;
      },
    },
    hasPointerCapture: { configurable: true, value: () => captured },
    releasePointerCapture: {
      configurable: true,
      value: () => {
        captured = false;
        release();
      },
    },
  });
});
afterEach(() => {
  for (const key of ['setPointerCapture', 'hasPointerCapture', 'releasePointerCapture'])
    Reflect.deleteProperty(HTMLElement.prototype, key);
});

it('keeps ordinary clicks and touch scrolling separate from a single committed pointer drag', () => {
  render(<Harness />);
  const button = screen.getByRole('button');
  pointer(button, 'pointerdown', 10);
  pointer(window, 'pointermove', 13);
  pointer(window, 'pointerup', 13);
  fireEvent.click(button, { detail: 1 });
  expect(choose).toHaveBeenCalledTimes(1);
  expect(preview).not.toHaveBeenCalled();
  pointer(button, 'pointerdown', 10, 'touch');
  pointer(window, 'pointermove', 80, 'touch');
  pointer(window, 'pointerup', 80, 'touch');
  expect(preview).not.toHaveBeenCalled();
  pointer(button, 'pointerdown', 10);
  pointer(window, 'pointermove', 80);
  expect(commit).not.toHaveBeenCalled();
  pointer(window, 'pointerup', 80);
  fireEvent.click(button, { detail: 1 });
  expect(commit).toHaveBeenCalledTimes(1);
  expect(commit.mock.calls[0]![0]).toMatchObject({
    kind: 'decoration',
    surface: { type: 'floor' },
  });
  expect(commit.mock.calls[0]![1]).toBe(pack);
  expect(choose).toHaveBeenCalledTimes(1);
  expect(captured).toBe(false);
});

it.each(['invalid', 'outside', 'escape', 'pointercancel', 'lostpointercapture', 'blur'])(
  'cancels %s with no commit and allows the next gesture',
  (reason) => {
    render(<Harness />);
    const button = screen.getByRole('button');
    pointer(button, 'pointerdown', 10);
    pointer(window, 'pointermove', 80);
    if (reason === 'invalid')
      preview.mockImplementation((object) => ({ object, problem: 'Keep the entrance clear' }));
    if (reason === 'outside') preview.mockReturnValue(undefined!);
    if (reason === 'escape') fireEvent.keyDown(window, { key: 'Escape' });
    if (reason === 'blur') fireEvent(window, new Event('blur'));
    if (reason === 'pointercancel' || reason === 'lostpointercapture') pointer(window, reason, 80);
    pointer(window, 'pointerup', 80);
    fireEvent.click(button, { detail: 1 });
    expect(commit).not.toHaveBeenCalled();
    expect(choose).not.toHaveBeenCalled();
    expect(captured).toBe(false);
    expect(screen.getByRole('status').textContent).toBe('');
    preview.mockImplementation((object) => ({ object, problem: undefined }));
    pointer(button, 'pointerdown', 10);
    pointer(window, 'pointermove', 80);
    pointer(window, 'pointerup', 80);
    expect(commit).toHaveBeenCalledTimes(1);
  }
);

it('clears capture and preview when disabled or disposed without a write', () => {
  const view = render(<Harness />);
  pointer(screen.getByRole('button'), 'pointerdown', 10);
  pointer(window, 'pointermove', 80);
  view.rerender(<Harness disabled />);
  expect(captured).toBe(false);
  pointer(window, 'pointerup', 80);
  expect(commit).not.toHaveBeenCalled();
  view.rerender(<Harness />);
  pointer(screen.getByRole('button'), 'pointerdown', 10);
  pointer(window, 'pointermove', 80);
  view.unmount();
  expect(captured).toBe(false);
  pointer(window, 'pointerup', 80);
  expect(commit).not.toHaveBeenCalled();
});

it('does not suppress keyboard activation after cancelling a pointer drag', () => {
  render(<Harness />);
  const button = screen.getByRole('button');
  pointer(button, 'pointerdown', 10);
  pointer(window, 'pointermove', 80);
  fireEvent.keyDown(window, { key: 'Escape' });
  fireEvent.click(button, { detail: 0 });
  expect(choose).toHaveBeenCalledTimes(1);
  expect(commit).not.toHaveBeenCalled();
});
