import { expect, it } from 'vite-plus/test';
import { busyFrames, createActionMark } from '../src/static';

class Node {
  className = '';
  dataset: Record<string, string> = {};
  attributes = new Map<string, string>();
  children: Node[] = [];
  constructor(readonly tag: string) {}
  setAttribute(name: string, value: string) {
    this.attributes.set(name, value);
  }
  append(child: Node) {
    this.children.push(child);
  }
}
const doc = {
  createElement: (tag: string) => new Node(tag),
  createElementNS: (_space: string, tag: string) => new Node(tag),
} as unknown as Document;

it('builds the reserved, hidden busy-mark slot with the leaf braille mark', () => {
  const mark = createActionMark(doc) as unknown as Node;
  expect(mark.tag).toBe('span');
  expect(mark.className).toBe('tmt-ui-action-mark');
  expect(mark.attributes.get('aria-hidden')).toBe('true');
  expect(mark.dataset.busy).toBe('false');
  // The same markup BusyMark renders: busy mark > frames > one span per braille frame.
  const busy = mark.children[0]!;
  expect(busy.className).toBe('tmt-ui-busy-mark');
  const frames = busy.children[0]!;
  expect(frames.className).toBe('tmt-ui-busy-frames');
  expect(frames.children.map((frame) => frame.dataset.frame)).toEqual([...busyFrames]);
});
