import { expect, it } from 'vite-plus/test';
import { createActionMark } from '../src/static';

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

it('builds the reserved, hidden busy-mark slot with the shared arc', () => {
  const mark = createActionMark(doc) as unknown as Node;
  expect(mark.tag).toBe('span');
  expect(mark.className).toBe('tmt-ui-action-mark');
  expect(mark.attributes.get('aria-hidden')).toBe('true');
  expect(mark.dataset.busy).toBe('false');
  const svg = mark.children[0]!;
  expect(svg.tag).toBe('svg');
  expect(svg.attributes.get('stroke')).toBe('currentColor');
  expect(svg.children[0]!.attributes.get('d')).toBe('M21 12a9 9 0 1 1-6.219-8.56');
});
