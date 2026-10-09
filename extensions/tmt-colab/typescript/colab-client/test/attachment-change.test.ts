import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import { digest, text } from '../src/index.js';
import {
  applyDocumentChange,
  attachmentDescriptor,
  documentChange,
  emptyDocumentChange,
  type AttachmentDescriptor,
} from '../src/attachment.js';

const oracle = JSON.parse(
  readFileSync(
    new URL('../../../contracts/vectors/attachment-change-v1.json', import.meta.url),
    'utf8',
  ),
);
const hex = (bytes: Uint8Array) => [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
const named: Record<string, Record<string, unknown>> = oracle.descriptors;
const descriptor = (key: string) => attachmentDescriptor(named[key]);
/** The oracle's `{"generated": n}` fillers: ids 0x1000 + i on the shape of `a`. */
function generated(count: number): AttachmentDescriptor[] {
  return Array.from({ length: count }, (_, i) => {
    const n = 0x1000 + i;
    return attachmentDescriptor({
      ...named.a,
      attachmentId: `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`,
      objectId: (n & 0xff).toString(16).padStart(2, '0').repeat(32),
      filename: `file-${n}.bin`,
    });
  });
}

interface OracleCase {
  name: string;
  current: string[] | { generated: number };
  set: string[];
  remove: string[];
  result: string | string[];
}

it('applies the typed change exactly as the shared oracle says', async () => {
  const source = hex(await digest(text(oracle.source)));
  expect(source).toBe(oracle.sourceSha256);
  for (const c of oracle.cases as OracleCase[]) {
    const current = Array.isArray(c.current)
      ? c.current.map(descriptor)
      : generated(c.current.generated);
    const change = {
      ...(c.set.length ? { set: c.set.map(descriptor) } : {}),
      ...(c.remove.length ? { remove: c.remove } : {}),
    };
    if (c.result === 'refused') {
      expect(() => applyDocumentChange(current, change, source), c.name).toThrow();
    } else {
      expect(
        applyDocumentChange(current, change, source).map((d) => d.attachmentId),
        c.name,
      ).toEqual(c.result);
    }
  }
});

it('admits only the strict set/remove shape and reports an empty change', () => {
  expect(emptyDocumentChange(documentChange({}))).toBe(true);
  expect(emptyDocumentChange(documentChange({ set: [], remove: [] }))).toBe(true);
  for (const bad of [
    null,
    [],
    { meta: { title: 'x' } },
    { set: {} },
    { remove: 'not-a-list' },
    { remove: ['not-an-id'] },
    { set: [{ attachmentId: 'x' }] },
    { remove: Array.from({ length: 129 }, (_, i) => `00000000-0000-4000-8000-${i.toString(16).padStart(12, '0')}`) },
  ])
    expect(() => documentChange(bad)).toThrow();
});
