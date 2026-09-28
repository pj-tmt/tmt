import { describe, expect, it } from 'vitest';
import example from '../../../../../contracts/whiteboard-scene-v1.json';
import {
  decodeWhiteboardDocument,
  decodeWhiteboardReceipt,
  decodeWhiteboardSave,
} from './document-contract.js';

const id = '11111111-1111-4111-8111-111111111111';
const document = { id: 'lobby', revision: 1, updatedAtMs: 100, scene: example };
const receipt = {
  documentId: 'lobby',
  operationId: id,
  revision: 1,
  changed: true,
  updatedAtMs: 100,
};

describe('whiteboard document envelopes', () => {
  it('admits blank virtual documents for valid IDs at revision zero, without inventing a persisted resource', () => {
    const absent = {
      ...document,
      revision: 0,
      updatedAtMs: 0,
      scene: { ...example, background: '#fff7e7', elements: [] },
    };
    expect(decodeWhiteboardDocument(absent)).toEqual(absent);
    expect(decodeWhiteboardDocument({ ...absent, id })).toEqual({ ...absent, id });
    for (const patch of [{ id: '../invalid' }, { updatedAtMs: 1 }, { scene: example }])
      expect(() => decodeWhiteboardDocument({ ...absent, ...patch })).toThrow();
    expect(decodeWhiteboardDocument(document)).toEqual(document);
    expect(decodeWhiteboardDocument({ ...document, id })).toMatchObject({ id });
  });

  it('rejects malformed, unsafe and foreign envelopes rather than filling missing fields', () => {
    for (const patch of [
      { id: '../other' },
      { revision: -1 },
      { revision: 0.5 },
      { revision: 2 ** 53 },
      { updatedAtMs: 0 },
      { updatedAtMs: null },
      { extra: true },
    ])
      expect(() => decodeWhiteboardDocument({ ...document, ...patch })).toThrow();
    expect(decodeWhiteboardReceipt(receipt)).toEqual(receipt);
    for (const patch of [
      { operationId: 'anything' },
      { changed: 1 },
      { revision: 0 },
      { updatedAtMs: 0 },
      { actor: 'alice' },
    ])
      expect(() => decodeWhiteboardReceipt({ ...receipt, ...patch })).toThrow();
    const save = { expectedRevision: 0, operationId: id, scene: example };
    expect(decodeWhiteboardSave(save)).toEqual(save);
    for (const patch of [
      { expectedRevision: 2 ** 53 },
      { expectedRevision: -1 },
      { actor: 'alice' },
      { scene: null },
    ])
      expect(() => decodeWhiteboardSave({ ...save, ...patch })).toThrow();
  });

  it('owns the decoded scene independently of the caller and other reads', () => {
    const input = structuredClone(document);
    const decoded = decodeWhiteboardDocument(input);
    const saved = decodeWhiteboardSave({
      expectedRevision: 1,
      operationId: id,
      scene: input.scene,
    });
    input.scene.elements.length = 0;
    decoded.scene.elements.reverse();
    expect(saved.scene).toEqual(example);
  });
});
