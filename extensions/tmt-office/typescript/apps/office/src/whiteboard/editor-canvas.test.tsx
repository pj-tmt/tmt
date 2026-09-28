import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { WhiteboardCanvas } from './editor-canvas.js';
import type { WhiteboardTool } from './editor-canvas.js';
import { decodeWhiteboardScene } from './scene-contract.js';
import example from '../../../../../contracts/whiteboard-scene-v1.json';
import { drawStrokeSegment, drawWhiteboard } from './drawing.js';

vi.mock('./drawing.js', () => ({
  drawWhiteboard: vi.fn(),
  drawWhiteboardSelection: vi.fn(),
  drawStrokeSegment: vi.fn(),
}));
const originalCanvasDescriptors = new Map(
  ['getContext', 'setPointerCapture', 'releasePointerCapture'].map((key) => [
    key,
    Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, key),
  ])
);

beforeEach(() => {
  vi.clearAllMocks();
  class Pointer extends MouseEvent {
    pointerId: number;
    constructor(type: string, input: PointerEventInit = {}) {
      super(type, input);
      this.pointerId = input.pointerId ?? 1;
    }
  }
  vi.stubGlobal('PointerEvent', Pointer);
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      disconnect() {}
    }
  );
  // Painting is mocked here; real 2D pixels are verified by the native browser scenario.
  Object.defineProperty(HTMLCanvasElement.prototype, 'getContext', {
    configurable: true,
    value: () => ({}),
  });
  vi.spyOn(HTMLCanvasElement.prototype, 'getBoundingClientRect').mockReturnValue({
    x: 0,
    y: 0,
    left: 0,
    top: 0,
    right: 800,
    bottom: 500,
    width: 800,
    height: 500,
    toJSON: () => ({}),
  });
  Object.defineProperty(HTMLCanvasElement.prototype, 'setPointerCapture', {
    configurable: true,
    value: vi.fn(),
  });
  Object.defineProperty(HTMLCanvasElement.prototype, 'releasePointerCapture', {
    configurable: true,
    value: vi.fn(),
  });
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  for (const [key, descriptor] of originalCanvasDescriptors) {
    if (descriptor) Object.defineProperty(HTMLCanvasElement.prototype, key, descriptor);
    else Reflect.deleteProperty(HTMLCanvasElement.prototype, key);
  }
});

describe('whiteboard pointer transaction boundary', () => {
  function setup(tool: WhiteboardTool) {
    const commit = vi.fn();
    const select = vi.fn();
    const scene = decodeWhiteboardScene({
      ...example,
      elements: tool === 'erase' || tool === 'select' ? example.elements : [],
    });
    render(
      <WhiteboardCanvas
        scene={scene}
        tool={tool}
        color="#285954"
        disabled={false}
        select={select}
        commit={commit}
      />
    );
    return { canvas: screen.getByLabelText('Whiteboard drawing surface'), commit, scene };
  }
  it.each<WhiteboardTool>([
    'stroke',
    'arrow',
    'rectangle',
    'ellipse',
    'note',
    'text',
    'erase',
    'select',
  ])('cancels %s without submitting an edit', (tool) => {
    const { canvas, commit } = setup(tool);
    fireEvent.pointerDown(canvas, { pointerId: 7, clientX: 100, clientY: 100, button: 0 });
    fireEvent.pointerMove(canvas, { pointerId: 7, clientX: 180, clientY: 150 });
    expect(commit).not.toHaveBeenCalled();
    fireEvent.pointerCancel(canvas, { pointerId: 7 });
    fireEvent.pointerUp(canvas, { pointerId: 7, clientX: 180, clientY: 150 });
    expect(commit).not.toHaveBeenCalled();
  });
  it('commits a drag exactly once with stable document coordinates, never on moves', () => {
    const { canvas, commit } = setup('rectangle');
    fireEvent.pointerDown(canvas, { pointerId: 7, clientX: 100, clientY: 100, button: 0 });
    for (const x of [120, 140, 160])
      fireEvent.pointerMove(canvas, { pointerId: 7, clientX: x, clientY: 150 });
    expect(commit).not.toHaveBeenCalled();
    fireEvent.pointerUp(canvas, { pointerId: 7, clientX: 160, clientY: 150 });
    expect(commit).toHaveBeenCalledTimes(1);
    expect(commit.mock.calls[0]![0].elements).toEqual([
      expect.objectContaining({ kind: 'rectangle', x: 200, y: 200, width: 120, height: 100 }),
    ]);
    fireEvent.lostPointerCapture(canvas, { pointerId: 7 });
    expect(commit).toHaveBeenCalledTimes(1);
  });
  it('ignores unrelated pointers and Escape cancels an owned gesture', () => {
    const { canvas, commit } = setup('stroke');
    fireEvent.pointerDown(canvas, { pointerId: 7, clientX: 100, clientY: 100, button: 0 });
    fireEvent.pointerUp(canvas, { pointerId: 9, clientX: 180, clientY: 150 });
    expect(commit).not.toHaveBeenCalled();
    fireEvent.keyDown(canvas, { key: 'Escape' });
    fireEvent.pointerUp(canvas, { pointerId: 7, clientX: 180, clientY: 150 });
    expect(commit).not.toHaveBeenCalled();
  });

  it('paints only new pen segments during a long drag and submits the scene once', () => {
    const { canvas, commit } = setup('stroke');
    fireEvent.pointerDown(canvas, { pointerId: 7, clientX: 100, clientY: 100, button: 0 });
    const fullPaints = vi.mocked(drawWhiteboard).mock.calls.length;
    for (let index = 1; index <= 1000; index++)
      fireEvent.pointerMove(canvas, {
        pointerId: 7,
        clientX: 100 + (index % 400),
        clientY: 100 + (index % 300),
      });
    expect(commit).not.toHaveBeenCalled();
    expect(drawWhiteboard).toHaveBeenCalledTimes(fullPaints);
    expect(drawStrokeSegment).toHaveBeenCalledTimes(1000);
    fireEvent.pointerUp(canvas, { pointerId: 7, clientX: 300, clientY: 200 });
    expect(commit).toHaveBeenCalledTimes(1);
    expect(commit.mock.calls[0]![0].elements[0].points).toHaveLength(1001);
  });
});
