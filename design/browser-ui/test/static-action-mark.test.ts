import { expect, it } from 'vite-plus/test';
import { busyDots, createActionMark } from '../src/static';

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

it('builds the reserved, hidden busy-mark slot with the leaf dot snake', () => {
  const mark = createActionMark(doc) as unknown as Node;
  expect(mark.tag).toBe('span');
  expect(mark.className).toBe('tmt-ui-action-mark');
  expect(mark.attributes.get('aria-hidden')).toBe('true');
  expect(mark.dataset.busy).toBe('false');
  // The same markup BusyMark renders: busy mark > svg > one circle per dot, classed, no style.
  const busy = mark.children[0]!;
  expect(busy.className).toBe('tmt-ui-busy-mark');
  const svg = busy.children[0]!;
  expect(svg.tag).toBe('svg');
  expect(svg.attributes.get('viewBox')).toBe('0 0 16 16');
  expect(svg.children.map((dot) => [dot.attributes.get('cx'), dot.attributes.get('cy')])).toEqual(
    busyDots.map(([cx, cy]) => [String(cx), String(cy)]),
  );
  expect(svg.children.every((dot) => dot.attributes.get('class') === 'tmt-ui-busy-dot')).toBe(true);
  expect(svg.children.every((dot) => !dot.attributes.has('style'))).toBe(true);
});
