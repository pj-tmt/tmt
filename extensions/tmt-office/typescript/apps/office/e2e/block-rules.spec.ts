import { test, expect } from '@playwright/test';
import { doc, getDocFromServer, setDoc, serverTimestamp, deleteDoc } from 'firebase/firestore';
import vectors from '../../../../../../contracts/office/block-v1.vectors.json' with { type: 'json' };
import { createWorldPort } from '../src/worlds/firebase-worlds.js';
import { createBlockPort } from '../src/blocks/firebase-blocks.js';
import { builtinFurniture } from '../src/blocks/block-contract.js';
import { createFirestoreFixture, setTester } from './firestore-fixture.js';

test('direct block writes enforce contract vectors and every bounded slot', async () => {
  const fixture = await createFirestoreFixture();
  try {
    const alice = await fixture.client(true);
    const worlds = createWorldPort(alice.db);
    const draft = worlds.draft('Rules workshop');
    await worlds.create(draft, alice.uid);
    const target = doc(alice.db, 'worlds', draft.id, 'blocks', 'home');
    let revision = 0;
    const valid = 'd000';
    const cases = [
      ...vectors.map((v) => ({ ...v, objects: [v.stored] })),
      { name: 'empty', valid: true, objects: [] },
      { name: 'full', valid: true, objects: Array(16).fill(valid) },
      {
        name: 'invalid final slot',
        valid: false,
        objects: [...Array(15).fill(valid), 'd0v0'],
      },
      { name: 'overflow', valid: false, objects: Array(17).fill(valid) },
    ];
    for (const vector of cases) {
      const writing = setDoc(target, {
        version: 1,
        revision: revision + 1,
        objects: vector.objects,
        updatedAt: serverTimestamp(),
      });
      if (vector.valid) {
        await writing;
        revision++;
        expect((await getDocFromServer(target)).data()?.objects, vector.name).toEqual(
          vector.objects
        );
      } else
        await expect(writing, vector.name).rejects.toMatchObject({ code: 'permission-denied' });
    }
    for (const changed of [
      { revision },
      { revision: revision + 2 },
      { version: 2 },
      { ownerUid: alice.uid },
      { updatedAt: 0 },
      { extra: true },
    ]) {
      await expect(
        setDoc(target, {
          version: 1,
          revision: revision + 1,
          objects: [],
          updatedAt: serverTimestamp(),
          ...changed,
        })
      ).rejects.toMatchObject({ code: 'permission-denied' });
    }
    await expect(deleteDoc(target)).rejects.toMatchObject({ code: 'permission-denied' });
    // Creation must support the full budget too, not only incremental updates.
    const full = worlds.draft('Full from the start');
    await worlds.create(full, alice.uid);
    const fullTarget = doc(alice.db, 'worlds', full.id, 'blocks', 'home');
    await setDoc(fullTarget, {
      version: 1,
      revision: 1,
      objects: Array(16).fill(valid),
      updatedAt: serverTimestamp(),
    });
    expect((await getDocFromServer(fullTarget)).data()?.objects).toHaveLength(16);
  } finally {
    await fixture.dispose();
  }
});

test('block transactions preserve exact retries and reject concurrent revision conflicts', async () => {
  const fixture = await createFirestoreFixture();
  try {
    const alice = await fixture.client(true);
    const worlds = createWorldPort(alice.db);
    const draft = worlds.draft('Concurrent studio');
    await worlds.create(draft, alice.uid);
    const port = createBlockPort(alice.db);
    const first = await port.apply(draft.id, 0, []);
    expect(first.revision).toBe(1);
    expect(await port.apply(draft.id, 0, [])).toEqual(first);
    const changes = [[builtinFurniture('desk', 0, 0, 0)], [builtinFurniture('plant', 0, 0, 0)]];
    const results = await Promise.allSettled(
      changes.map((objects) => port.apply(draft.id, 1, objects))
    );
    expect(results.filter((result) => result.status === 'fulfilled')).toHaveLength(1);
    const failed = results.find((result) => result.status === 'rejected');
    expect(failed?.status === 'rejected' && failed.reason.message).toContain('changed');
    const stored = (
      await getDocFromServer(doc(alice.db, 'worlds', draft.id, 'blocks', 'home'))
    ).data();
    expect(stored?.revision).toBe(2);
    expect(stored?.objects).toEqual(results[0].status === 'fulfilled' ? ['d000'] : ['p000']);
  } finally {
    await fixture.dispose();
  }
});

test('only the approved owner can access the block, including after revocation', async () => {
  const fixture = await createFirestoreFixture();
  try {
    const alice = await fixture.client(true);
    const bob = await fixture.client(true);
    const device = await fixture.client(true, 'device');
    const waiting = await fixture.client();
    const worlds = createWorldPort(alice.db);
    const draft = worlds.draft('Private studio');
    await worlds.create(draft, alice.uid);
    await createBlockPort(alice.db).apply(draft.id, 0, []);
    for (const actor of [bob, device, waiting]) {
      await expect(
        getDocFromServer(doc(actor.db, 'worlds', draft.id, 'blocks', 'home'))
      ).rejects.toMatchObject({ code: 'permission-denied' });
      await expect(createBlockPort(actor.db).apply(draft.id, 1, [])).rejects.toMatchObject({
        code: 'permission-denied',
      });
    }
    await setTester(alice.uid, false);
    await expect(createBlockPort(alice.db).apply(draft.id, 1, [])).rejects.toMatchObject({
      code: 'permission-denied',
    });
    await expect(
      getDocFromServer(doc(alice.db, 'worlds', draft.id, 'blocks', 'home'))
    ).rejects.toMatchObject({ code: 'permission-denied' });
  } finally {
    await fixture.dispose();
  }
});
