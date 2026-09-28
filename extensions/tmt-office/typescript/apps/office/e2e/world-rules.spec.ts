import { test, expect } from '@playwright/test';
import { signOut } from 'firebase/auth';
import {
  collection,
  deleteDoc,
  doc,
  getDocFromServer,
  getDocsFromServer,
  serverTimestamp,
  setDoc,
  updateDoc,
  onSnapshot,
} from 'firebase/firestore';
import { createWorldPort } from '../src/worlds/firebase-worlds.js';
import {
  createFirestoreFixture,
  setTester,
  writeTesterFields,
  updateWorldName,
} from './firestore-fixture.js';

test('missing, malformed and disabled tester grants deny access until repaired', async () => {
  const fixture = await createFirestoreFixture();
  try {
    const alice = await fixture.client(true);
    const port = createWorldPort(alice.db);
    const draft = port.draft('Grant policy');
    await port.create(draft, alice.uid);
    const reference = doc(alice.db, 'worlds', draft.id);
    for (const fields of [
      {},
      { enabled: { stringValue: 'true' } },
      { enabled: { booleanValue: false } },
      { enabled: { booleanValue: true }, extra: { booleanValue: true } },
    ]) {
      await writeTesterFields(alice.uid, fields);
      await expect(getDocFromServer(reference)).rejects.toMatchObject({
        code: 'permission-denied',
      });
    }
    await setTester(alice.uid, true);
    expect((await getDocFromServer(reference)).data()?.name).toBe('Grant policy');
  } finally {
    await fixture.dispose();
  }
});

test('server listeners reject cross-owner access and withhold updates after revocation without token refresh', async () => {
  const fixture = await createFirestoreFixture();
  const stops: Array<() => void> = [];
  try {
    const alice = await fixture.client(true);
    const bob = await fixture.client(true);
    const port = createWorldPort(alice.db);
    const draft = port.draft('Stream policy');
    await port.create(draft, alice.uid);
    const seen: string[] = [];
    const bobSeen: unknown[] = [];
    let aliceError: string | undefined;
    let bobError: string | undefined;
    stops.push(
      onSnapshot(
        doc(alice.db, 'worlds', draft.id),
        { includeMetadataChanges: true },
        (snapshot) => {
          if (!snapshot.metadata.fromCache && snapshot.exists()) seen.push(snapshot.data().name);
        },
        (error) => {
          aliceError = error.code;
        }
      )
    );
    stops.push(
      onSnapshot(
        doc(bob.db, 'worlds', draft.id),
        (snapshot) => {
          if (snapshot.exists()) bobSeen.push(snapshot.data());
        },
        (error) => {
          bobError = error.code;
        }
      )
    );
    await expect.poll(() => seen).toContain('Stream policy');
    await expect.poll(() => bobError).toBe('permission-denied');
    expect(bobSeen).toEqual([]);
    await setTester(alice.uid, false);
    // The emulator does not immediately close listeners on a dependent grant
    // change. Prove it cannot disclose the next world update, independently of UI.
    await updateWorldName(draft.id, 'Must remain private');
    await expect.poll(() => aliceError).toBe('permission-denied');
    expect(seen.every((name) => name === 'Stream policy')).toBe(true);
  } finally {
    for (const stop of stops) stop();
    await fixture.dispose();
  }
});

test('valid anonymous, custom-device and unverified Google sessions cannot become approved humans', async () => {
  const fixture = await createFirestoreFixture();
  try {
    for (const provider of ['anonymous', 'device', 'unverified'] as const) {
      const client = await fixture.client(true, provider);
      expect(client.auth.currentUser?.uid).toBe(client.uid);
      const token = await client.auth.currentUser!.getIdTokenResult();
      expect(token.signInProvider).toBe(
        provider === 'device' ? 'custom' : provider === 'anonymous' ? 'anonymous' : 'google.com'
      );
      await expect(getDocFromServer(doc(client.db, 'testers', client.uid))).rejects.toMatchObject({
        code: 'permission-denied',
      });
      const port = createWorldPort(client.db);
      await expect(port.create(port.draft('Denied'), client.uid)).rejects.toMatchObject({
        code: 'permission-denied',
      });
    }
  } finally {
    await fixture.dispose();
  }
});

test('online creation is immutable and concurrent exact retries preserve one world', async () => {
  const fixture = await createFirestoreFixture();
  try {
    const alice = await fixture.client(true);
    const port = createWorldPort(alice.db);
    const draft = port.draft('Alice office');
    expect(
      await Promise.all([port.create(draft, alice.uid), port.create(draft, alice.uid)])
    ).toEqual([draft.id, draft.id]);
    const reference = doc(alice.db, 'worlds', draft.id);
    const original = (await getDocFromServer(reference)).data();
    expect(original?.name).toBe('Alice office');
    expect(original?.ownerUid).toBe(alice.uid);
    expect(original?.createdAt.toMillis()).toBeGreaterThan(0);
    await port.create(draft, alice.uid);
    await expect(port.create({ ...draft, name: 'Conflict' }, alice.uid)).rejects.toThrow(
      'conflicts'
    );
    expect((await getDocFromServer(reference)).data()).toEqual(original);
    await expect(updateDoc(reference, { name: 'Overwritten' })).rejects.toMatchObject({
      code: 'permission-denied',
    });
    await expect(deleteDoc(reference)).rejects.toMatchObject({ code: 'permission-denied' });
  } finally {
    await fixture.dispose();
  }
});

test('tester admission never grants another owner access or lets clients grant themselves', async () => {
  const fixture = await createFirestoreFixture();
  try {
    const alice = await fixture.client(true);
    const bob = await fixture.client(true);
    const waiting = await fixture.client();
    const port = createWorldPort(alice.db);
    const draft = port.draft('Private');
    await port.create(draft, alice.uid);
    for (const client of [bob, waiting]) {
      await expect(getDocFromServer(doc(client.db, 'worlds', draft.id))).rejects.toMatchObject({
        code: 'permission-denied',
      });
      await expect(getDocsFromServer(collection(client.db, 'worlds'))).rejects.toMatchObject({
        code: 'permission-denied',
      });
      await expect(
        setDoc(doc(client.db, 'testers', client.uid), { enabled: true })
      ).rejects.toMatchObject({ code: 'permission-denied' });
      await expect(getDocFromServer(doc(client.db, 'testers', alice.uid))).rejects.toMatchObject({
        code: 'permission-denied',
      });
    }
    expect((await getDocFromServer(doc(waiting.db, 'testers', waiting.uid))).exists()).toBe(false);
    await expect(
      createWorldPort(waiting.db).create(port.draft('Denied'), waiting.uid)
    ).rejects.toMatchObject({ code: 'permission-denied' });
    await setTester(alice.uid, false);
    await expect(getDocFromServer(doc(alice.db, 'worlds', draft.id))).rejects.toMatchObject({
      code: 'permission-denied',
    });
    await signOut(bob.auth);
    await expect(getDocFromServer(doc(bob.db, 'worlds', draft.id))).rejects.toMatchObject({
      code: 'permission-denied',
    });
  } finally {
    await fixture.dispose();
  }
});

test('direct writes cannot bypass shape, ownership, name or server-time validation', async () => {
  const fixture = await createFirestoreFixture();
  try {
    const alice = await fixture.client(true);
    const valid = () => ({
      version: 1,
      name: 'Valid',
      ownerUid: alice.uid,
      createdAt: serverTimestamp(),
    });
    for (const changes of [
      { ownerUid: 'forged' },
      { version: 2 },
      { extra: true },
      { name: '' },
      { name: '   ' },
      { name: '\u00a0' },
      { name: '\u3000' },
      { name: 'x'.repeat(81) },
      { name: 'a\nb' },
      { name: 42 },
      { createdAt: 0 },
    ]) {
      await expect(
        setDoc(doc(collection(alice.db, 'worlds')), { ...valid(), ...changes })
      ).rejects.toMatchObject({ code: 'permission-denied' });
    }
    const reference = doc(collection(alice.db, 'worlds'));
    await setDoc(reference, { ...valid(), name: 'x'.repeat(80) });
    expect((await getDocFromServer(reference)).data()?.name).toBe('x'.repeat(80));
    const unicode = doc(collection(alice.db, 'worlds'));
    await setDoc(unicode, { ...valid(), name: '😀'.repeat(80) });
    expect((await getDocFromServer(unicode)).data()?.name).toBe('😀'.repeat(80));
    await expect(
      setDoc(doc(reference, 'blocks', 'agent'), { name: 'Agent' })
    ).rejects.toMatchObject({ code: 'permission-denied' });
  } finally {
    await fixture.dispose();
  }
});
