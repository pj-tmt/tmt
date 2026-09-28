import { expect, it, vi } from 'vitest';
import { Container } from 'pixi.js';
import { drawSceneComponents } from './scene-components.js';
import type { SceneComponent } from './scene-component-geometry.js';

// Observe ordering and measured-label geometry without a GPU. Real pointer,
// text and pixel behavior is covered by the native spatial browser scenarios.
vi.mock('pixi.js', () => {
  class Container {
    children: unknown[] = [];
    position = { set: vi.fn() };
    addChild(child: unknown) {
      this.children = this.children.filter((value) => value !== child);
      this.children.push(child);
      return child;
    }
  }
  class Graphics {
    clear() {
      return this;
    }
    rect() {
      return this;
    }
    roundRect() {
      return this;
    }
    fill() {
      return this;
    }
    stroke() {
      return this;
    }
  }
  return { Container, Graphics };
});
vi.mock('./scene-label.js', () => ({
  sceneLabel: () => ({
    text: '',
    style: {},
    scale: { set: vi.fn() },
    position: { set: vi.fn() },
    anchor: { set: vi.fn() },
    height: 5,
  }),
}));

const component = (id: string, x: number): SceneComponent => ({
  id,
  label: `Open ${id}`,
  available: true,
  bounds: { x, y: 3, width: 12, height: 8 },
});

it('keeps expanded-label paint order and hit order identical, restoring both when idle', () => {
  const parent = new Container();
  // Expanded labels extend left from the object's right edge. Stagger the
  // neighbor vertically so its idle badge overlaps that expanded label.
  const neighbor = component('whiteboard', 1);
  neighbor.bounds.y = -4;
  const view = drawSceneComponents(parent, [component('board', 14), neighbor]);
  const layer = parent.children[0] as Container;
  const initial = [...layer.children];
  view.zoom(4);
  expect(view.pick(11, 0)).toBe('whiteboard');
  expect(view.highlight('board')).toBe(true);
  expect(layer.children).toEqual([initial[1], initial[0]]);
  expect(view.pick(11, 0)).toBe('board');
  // Measured five-tile text plus padding, not the old fixed-height plaque.
  expect(view.pick(-10, -4)).toBe('board');
  expect(view.pick(-10, -5)).toBeUndefined();
  expect(view.highlight('board')).toBe(false);
  expect(view.highlight()).toBe(true);
  expect(layer.children).toEqual(initial);
  expect(view.pick(11, 0)).toBe('whiteboard');
  expect(view.pick(-10, -4)).toBeUndefined();
});

it('updates the same hit rectangles when zoom changes an expanded label', () => {
  const view = drawSceneComponents(new Container(), [component('board', 1)]);
  view.zoom(4);
  view.highlight('board');
  expect(view.pick(-20, 0)).toBe('board');
  view.zoom(8);
  expect(view.pick(-20, 0)).toBeUndefined();
  expect(view.pick(0, 0)).toBe('board');
});
