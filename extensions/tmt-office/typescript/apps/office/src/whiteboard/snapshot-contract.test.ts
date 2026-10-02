import { expect, it } from 'vite-plus/test';
import scene from '../../../../../contracts/whiteboard-scene-v1.json';
import { decodeWhiteboardCapture, decodeWhiteboardSnapshot } from './snapshot-contract.js';

const id = '11111111-1111-4111-8111-111111111111';
const input = {
  expectedRevision: 1,
  operationId: id,
  selectedElementIds: [],
  annotation: 'Inspect this.\n<script>inert</script>',
};
const captured = {
  id,
  documentId: 'lobby',
  documentRevision: 1,
  scene,
  selectedElementIds: [],
  annotation: input.annotation,
  createdAtMs: 100,
};

it('admits an owned capture and retained scene with canonical set selection', () => {
  const ids = scene.elements
    .slice(0, 2)
    .map((item) => item.id)
    .reverse();
  const original = [...ids];
  const result = decodeWhiteboardCapture({ ...input, selectedElementIds: ids });
  expect(result.selectedElementIds).toEqual([...ids].sort());
  expect(ids).toEqual(original);
  expect(decodeWhiteboardSnapshot({ ...captured, selectedElementIds: ids })).toEqual({
    ...captured,
    selectedElementIds: [...ids].sort(),
  });
  result.selectedElementIds.pop();
  expect(ids).toEqual(original);
});

it('rejects unsaved, malformed and actor-supplied captures and nonmember snapshot selections', () => {
  for (const patch of [
    { expectedRevision: 0 },
    { expectedRevision: 1.2 },
    { expectedRevision: Number.MAX_SAFE_INTEGER + 1 },
    { operationId: 'lobby' },
    { scene },
    { actor: 'alice' },
    { annotation: '\u0000' },
    { annotation: 'x'.repeat(16385) },
    { selectedElementIds: [id, id] },
  ])
    expect(() => decodeWhiteboardCapture({ ...input, ...patch })).toThrow();
  for (const patch of [
    { id: 'lobby' },
    { createdAtMs: 0 },
    { documentRevision: 0 },
    { documentId: '../other' },
    { selectedElementIds: ['77777777-7777-4777-8777-777777777777'] },
    { extra: true },
  ])
    expect(() => decodeWhiteboardSnapshot({ ...captured, ...patch })).toThrow();
  expect(decodeWhiteboardCapture(input).annotation).toBe(input.annotation);
  expect(decodeWhiteboardSnapshot(captured).scene).toEqual(scene);
});
