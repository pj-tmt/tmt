import { afterEach, expect, it } from 'vite-plus/test';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { WorldYjsDocument } from './world-yjs-document.js';
import type { WorldDocument } from './world-contract.js';

const origin = Symbol('test');
const documents: WorldYjsDocument[] = [];
afterEach(() => documents.splice(0).forEach((document) => document.destroy()));
function fixture() {
  const world = officeWorldFixture().layout;
  const document = new WorldYjsDocument(world, origin);
  documents.push(document);
  return { world, document };
}

it('publishes entity values and ordering together in one complete update', () => {
  const { world, document } = fixture();
  const observed: WorldDocument[] = [];
  const unsubscribe = document.onUpdate(() => observed.push(document.snapshot()));
  const next = {
    ...world,
    objects: [...world.objects].reverse().map((object) => ({
      ...object,
      placement: { ...object.placement, x: object.placement.x + 1 },
    })),
  };
  document.replace(next, origin);
  expect(observed).toEqual([next]);
  document.replace(next, origin);
  expect(observed).toHaveLength(1);
  unsubscribe();
  document.replace(world, origin);
  expect(observed).toHaveLength(1);
});

it('rejects a partially valid batch without changing data, order or history', () => {
  const { world, document } = fixture();
  const history = document.createHistory(origin);
  let updates = 0;
  document.onUpdate(() => updates++);
  const next = {
    ...world,
    objects: world.objects.map((object) => ({
      ...object,
      placement: { ...object.placement, x: object.placement.x + 1 },
    })),
  };
  expect(() =>
    document.replace({ ...next, objects: [...next.objects, next.objects[0]!] }, origin)
  ).toThrow('Duplicate Office object ID');
  expect(document.snapshot()).toEqual(world);
  expect(updates).toBe(0);
  expect(history.canUndo()).toBe(false);
  history.destroy();
});

it('does not retain mutable aliases from inputs or expose stored values through snapshots', () => {
  const { world, document } = fixture();
  const expected = structuredClone(world);
  Reflect.set(world.objects[0]!.placement, 'x', 999);
  const read = document.snapshot();
  Reflect.set(read.objects[0]!.placement.footprint, 'width', 999);
  expect(document.snapshot()).toEqual(expected);
  document.replace(expected, origin);
  Reflect.set(expected.objects[0]!.placement, 'x', 888);
  expect(document.snapshot().objects[0]!.placement.x).not.toBe(888);
});
