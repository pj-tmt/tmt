import { describe, expect, it } from 'vite-plus/test';
import example from '../../../../../contracts/whiteboard-scene-v1.json';
import { decodeWhiteboardScene } from './scene-contract.js';
import { commitScene, createHistory, currentScene, redoScene, undoScene } from './history.js';

const scene = () => decodeWhiteboardScene(example);

describe('whiteboard edit transactions', () => {
  it('owns submitted values and restores element IDs and paint order across undo/redo', () => {
    const initial = scene();
    const history = createHistory(initial);
    initial.elements.reverse();
    expect(currentScene(history)).toEqual(example);
    const next = scene();
    next.elements.reverse();
    const edited = commitScene(history, next);
    next.elements.length = 0;
    expect(currentScene(edited).elements.map((element) => element.id)).toEqual(
      example.elements.map((element) => element.id).reverse()
    );
    expect(currentScene(undoScene(edited))).toEqual(example);
    expect(currentScene(redoScene(undoScene(edited)))).toEqual(currentScene(edited));
  });

  it('keeps no-ops out of history and branches only when a new edit completes after undo', () => {
    const first = createHistory(scene());
    expect(undoScene(first)).toBe(first);
    expect(redoScene(first)).toBe(first);
    expect(commitScene(first, scene())).toBe(first);
    const second = commitScene(first, { ...scene(), background: '#ffffff' });
    const undone = undoScene(second);
    expect(commitScene(undone, scene())).toBe(undone);
    expect(redoScene(undone).cursor).toBe(1);
    const branch = commitScene(undone, { ...scene(), elements: [] });
    expect(redoScene(branch)).toBe(branch);
    expect(currentScene(undoScene(branch))).toEqual(example);
  });

  it('rejects invalid completed edits without damaging the current scene or redo chain', () => {
    const edited = commitScene(createHistory(scene()), { ...scene(), elements: [] });
    const undone = undoScene(edited);
    const invalid = scene();
    invalid.elements[0]!.id = 'not-an-id';
    expect(() => commitScene(undone, invalid)).toThrow();
    expect(currentScene(undone)).toEqual(example);
    expect(currentScene(redoScene(undone)).elements).toEqual([]);
  });

  it('bounds retained snapshots without changing the current document', () => {
    let history = createHistory(scene());
    for (let index = 0; index < 100; index++)
      history = commitScene(history, {
        ...scene(),
        background: `#${index.toString(16).padStart(6, '0')}`,
      });
    expect(history.entries).toHaveLength(64);
    expect(currentScene(history).background).toBe('#000063');
    for (let index = 0; index < 100; index++) history = undoScene(history);
    expect(history.cursor).toBe(0);
    expect(currentScene(history).background).toBe('#000024');
  });

  it('evicts point-heavy history by its serialized budget before reaching the entry cap', () => {
    const large = decodeWhiteboardScene({
      ...example,
      elements: [
        {
          id: '11111111-1111-4111-8111-111111111111',
          kind: 'stroke',
          color: '#000000',
          strokeWidth: 2,
          points: Array.from({ length: 65536 }, () => [1000, 1000]),
        },
      ],
    });
    let history = createHistory(large);
    for (let index = 0; index < 25; index++)
      history = commitScene(history, {
        ...large,
        background: `#${index.toString(16).padStart(6, '0')}`,
      });
    expect(history.entries.length).toBeLessThan(26);
    expect(history.entries.reduce((total, item) => total + item.bytes, 0)).toBeLessThanOrEqual(
      16 * 1024 * 1024
    );
    expect(currentScene(history).elements).toEqual(large.elements);
    expect(currentScene(history).background).toBe('#000018');
  });
});
