import { readFileSync } from 'node:fs';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import * as Y from 'yjs';
import { attachment } from '@tmt/colab-client';
import type { DecoderCommand } from '../src/fold-protocol.js';

const oracle = JSON.parse(
  readFileSync(
    new URL('../../../contracts/vectors/attachment-change-v1.json', import.meta.url),
    'utf8',
  ),
);
const named: Record<string, Record<string, unknown>> = oracle.descriptors;
const descriptor = (key: string) => attachment.attachmentDescriptor(named[key]);
function generated(count: number) {
  return Array.from({ length: count }, (_, i) => {
    const n = 0x1000 + i;
    return attachment.attachmentDescriptor({
      ...named.a,
      attachmentId: `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`,
      objectId: (n & 0xff).toString(16).padStart(2, '0').repeat(32),
      filename: `file-${n}.bin`,
    });
  });
}
afterEach(() => vi.unstubAllGlobals());
/** The real decoder Worker module, driven in-process like the production message loop. */
async function worker() {
  const surface = {
    onmessage: null as unknown as (event: {
      data: { id: number; command: DecoderCommand };
    }) => Promise<void>,
    postMessage: vi.fn(),
  };
  vi.stubGlobal('self', surface);
  vi.resetModules();
  await import('../src/fold.worker.js');
  return async (command: DecoderCommand) => {
    surface.postMessage.mockClear();
    await surface.onmessage({ data: { id: 1, command } });
    const result = surface.postMessage.mock.calls[0][0];
    if (result.error) throw new Error(result.error);
    return result;
  };
}
function seed(source: string, list: unknown[]) {
  const doc = new Y.Doc();
  doc.getText('html').insert(0, source);
  doc.getMap('meta').set('title', 'Files');
  if (list.length) doc.getMap('meta').set('attachments', JSON.parse(JSON.stringify(list)));
  return Y.encodeStateAsUpdate(doc);
}
/** The admitted content a preparation starts from; an absent list stays absent. */
const snapshot = (view: Record<string, unknown>) => ({
  source: view.source as string,
  title: view.title as string,
  own: view.own as never,
  ...(Object.hasOwn(view, 'attachments') ? { attachments: view.attachments as never } : {}),
});
const ids = (value?: { attachmentId: string }[]) => (value ?? []).map((d) => d.attachmentId);

it('prepares a typed set/remove change in the Worker exactly as the shared oracle says', async () => {
  for (const c of oracle.cases) {
    const current = Array.isArray(c.current)
      ? c.current.map(descriptor)
      : generated(c.current.generated);
    const run = await worker();
    const base = await run({ type: 'apply', updates: [seed(oracle.source, current)] });
    const change = {
      ...(c.set.length ? { set: c.set.map(descriptor) } : {}),
      ...(c.remove.length ? { remove: c.remove } : {}),
    };
    const prepared = run({
      type: 'prepare-content',
      source: oracle.source,
      base: snapshot(base),
      attachments: change,
    });
    if (c.result === 'refused') await expect(prepared, c.name).rejects.toThrow();
    else {
      const result = await prepared;
      expect(ids(result.projection.attachments), c.name).toEqual(c.result);
    }
  }
});

it('replays the prepared deltas to the edited list, leaves the source alone and drops the key when empty', async () => {
  const run = await worker();
  let view = await run({ type: 'apply', updates: [seed(oracle.source, [])] });
  const step = async (change: attachment.DocumentChange) => {
    const result = await run({
      type: 'prepare-content',
      source: oracle.source,
      base: snapshot(view),
      attachments: change,
    });
    expect(result.kind).toBe('updates');
    view = await run({ type: 'apply', updates: result.updates });
    // A fresh replay of the log is exactly what the preparer said it would produce.
    expect(view.attachments).toEqual(result.projection.attachments);
    expect(view.source).toBe(oracle.source);
    return view;
  };
  expect(ids((await step({ set: [descriptor('a')] })).attachments)).toEqual(ids([descriptor('a')]));
  expect(
    ids(
      (await step({ set: [descriptor('b')], remove: [descriptor('a').attachmentId] })).attachments,
    ),
  ).toEqual(ids([descriptor('b')]));
  const none = await step({ remove: [descriptor('b').attachmentId] });
  expect(Object.hasOwn(none, 'attachments')).toBe(false);
});

it('an unchanged source with no change stays a noop', async () => {
  const run = await worker();
  const base = await run({ type: 'apply', updates: [seed(oracle.source, [descriptor('a')])] });
  const result = await run({
    type: 'prepare-content',
    source: oracle.source,
    base: snapshot(base),
  });
  expect(result.kind).toBe('noop');
});
