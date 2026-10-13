import { expect, test } from 'vite-plus/test';
import { verifyHostedDeclaration } from '../src/firebase-declaration.js';

async function declaration(chunkBytes?: unknown) {
  const bytes = JSON.stringify({
    version: 1,
    extension: 'notes',
    backend: 'firestore',
    resources: [
      {
        name: 'chunks',
        kind: 'blob',
        path: 'chunks',
        limits: {
          maxObjectBytes: 12582912,
          maxNamespaceBytes: 12582912,
          maxEntries: 513,
          ...(chunkBytes === undefined ? {} : { chunkBytes }),
        },
        ttlField: null,
        indexes: [],
      },
    ],
    admission: { artifact: 'admission.rules', digest: 'a'.repeat(64), entryPoint: 'admitted' },
  });
  const digest = Array.from(
    new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(bytes))),
    (b) => b.toString(16).padStart(2, '0'),
  ).join('');
  return { bytes, digest };
}
test('declared cloud chunk size is optional, bounded, and bound to exact bytes', async () => {
  await verifyHostedDeclaration(await declaration());
  for (const size of [32768, 262144]) await verifyHostedDeclaration(await declaration(size));
  for (const size of [0, -1, 1.5, null, '32768', 12582913, Number.MAX_SAFE_INTEGER + 1])
    await expect(verifyHostedDeclaration(await declaration(size))).rejects.toMatchObject({
      code: 'REMOTE_INPUT_INVALID',
    });
  const small = await declaration(32768),
    large = await declaration(262144);
  await expect(verifyHostedDeclaration({ ...large, digest: small.digest })).rejects.toMatchObject({
    code: 'REMOTE_INPUT_INVALID',
  });
});

import { readFileSync } from 'node:fs';
import {
  bindSyncContext,
  syncDocumentPath,
  verifySealedObject,
  type SealedObjectManifest,
  type SyncAuthPort,
} from '../src/sealed-sync.js';
const vectors = JSON.parse(
  readFileSync(
    new URL('../../../rust/tmt-remote/tests/fixtures/sealed_sync/v1.json', import.meta.url),
    'utf8',
  ),
) as {
  manifest: SealedObjectManifest;
  count: number;
  lastBytes: number;
  manifestBytes: string;
  ids: string[];
}[];
test('independent vectors pin complete-or-absent objects and both chunk sizes', async () => {
  for (const v of vectors) {
    const m = v.manifest;
    const data = Uint8Array.from({ length: m.bytes }, (_, i) => i % 251);
    const chunks: Uint8Array[] = [];
    for (let offset = 0; offset < data.length; offset += m.chunkBytes)
      chunks.push(data.slice(offset, offset + m.chunkBytes));
    await verifySealedObject('fixture-original', m, chunks, 12582912);
    expect(chunks.length).toBe(v.count);
    expect(chunks.at(-1)?.length ?? 0).toBe(v.lastBytes);
    expect(
      JSON.stringify(Object.fromEntries(Object.entries(m).sort(([a], [b]) => a.localeCompare(b)))),
    ).toBe(v.manifestBytes);
    expect(chunks.map((_, i) => `fixture-original+${i}`)).toEqual(v.ids);
    await expect(verifySealedObject('another-original', m, chunks, 12582912)).rejects.toMatchObject(
      { code: 'REMOTE_INPUT_INVALID' },
    );
    await expect(
      verifySealedObject('fixture-original', { ...m, digest: '0'.repeat(64) }, chunks, 12582912),
    ).rejects.toMatchObject({ code: 'REMOTE_INPUT_INVALID' });
    if (chunks.length)
      await expect(
        verifySealedObject('fixture-original', m, chunks.slice(0, -1), 12582912),
      ).rejects.toMatchObject({ code: 'REMOTE_INPUT_INVALID' });
  }
});
test('pure context binding and declared targets perform no token or provider call', async () => {
  const auth: SyncAuthPort = {
    identity: { projectId: 'demo-remote-1', uid: 'fixture-user' },
    token() {
      throw Error('must not acquire a token');
    },
  };
  const config = {
    apiKey: 'public',
    appId: '1:123:web:fixture',
    projectId: 'demo-remote-1',
    authDomain: 'demo-remote-1.firebaseapp.com',
  };
  const context = await bindSyncContext(config, await declaration(262144), 'fixture-space', auth);
  expect(syncDocumentPath(context, { resource: 'chunks', id: 'fixture-user+1' })).toBe(
    'x/notes/chunks/fixture-user+1',
  );
  for (const id of ['', '../escape', 'x/append', 'a b'])
    expect(() => syncDocumentPath(context, { resource: 'chunks', id })).toThrow();
  await expect(
    bindSyncContext(config, await declaration(), 'fixture-space', {
      ...auth,
      identity: { ...auth.identity, projectId: 'another-project' },
    }),
  ).rejects.toMatchObject({ code: 'REMOTE_SCOPE_DENIED' });
});
