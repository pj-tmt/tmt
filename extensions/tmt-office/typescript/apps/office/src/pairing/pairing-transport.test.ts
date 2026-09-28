import { expect, it, vi } from 'vitest';
import { createPairingPort } from './pairing-transport.js';
import { pairingEndpoint } from '../auth/firebase-config.js';
import type { PairingRequest } from './pairing-contract.js';

const pairingId = 'a'.repeat(64);
const endpoint = 'https://pair.example/officePairing';
const revokeRequest: PairingRequest = {
  version: 1,
  pairingId,
  worldId: 'a'.repeat(20),
  installationId: '00000000-0000-4000-8000-000000000001',
  identityId: '00000000-0000-4000-8000-000000000002',
  installationLabel: 'Device',
  identityLabel: 'Alice',
  capabilities: ['layout.read'],
};
const response = () =>
  new Response(JSON.stringify({ version: 1, pairingId, revoked: true }), {
    headers: { 'content-type': 'application/json' },
  });

it('sends the owner selector separately and rejects a different returned retained block', async () => {
  const id = '00000000-0000-4000-8000-000000000001';
  const request: PairingRequest = {
    version: 1,
    pairingId,
    worldId: 'a'.repeat(20),
    installationId: id,
    identityId: id,
    installationLabel: 'Device',
    identityLabel: 'Alice',
    capabilities: ['layout.read'],
  };
  const replacement = { principalUid: `office-agent:${id}`, blockId: id };
  const auth = { currentUser: { uid: 'owner', getIdToken: async () => 'private-token' } };
  const binding = {
    ...request,
    principalUid: 'office-agent:00000000-0000-4000-8000-000000000002',
    blockId: id,
    expiresAt: 2_000_000_000_000,
  };
  const send = vi
    .fn<typeof fetch>()
    .mockImplementation(
      async () =>
        new Response(JSON.stringify(binding), { headers: { 'content-type': 'application/json' } })
    );
  const port = createPairingPort(auth, endpoint, send);
  expect(await port.approve(request, 'owner', replacement)).toEqual(binding);
  expect(JSON.parse(send.mock.calls[0][1]!.body as string)).toEqual({
    ...request,
    replacesPrincipalUid: replacement.principalUid,
  });
  expect(request).not.toHaveProperty('replacesPrincipalUid');
  binding.blockId = '00000000-0000-4000-8000-000000000003';
  await expect(port.approve(request, 'owner', replacement)).rejects.toMatchObject({
    kind: 'uncertain',
  });
  binding.blockId = id;
  binding.principalUid = replacement.principalUid;
  await expect(port.approve(request, 'owner', replacement)).rejects.toMatchObject({
    kind: 'uncertain',
  });
  await expect(
    port.approve(request, 'owner', { ...replacement, principalUid: '../other' })
  ).rejects.toThrow();
  expect(send).toHaveBeenCalledTimes(3);
});

it('only explicit deployment settings select an endpoint, never request or preview data', () => {
  expect(pairingEndpoint('preview', { VITE_OFFICE_PAIRING_URL: endpoint })).toBeUndefined();
  expect(pairingEndpoint('cloud', {})).toBeUndefined();
  expect(pairingEndpoint('cloud', { VITE_OFFICE_PAIRING_URL: endpoint })).toBe(endpoint);
  expect(pairingEndpoint('emulator', { VITE_OFFICE_PAIRING_URL: endpoint })).toBe(
    'http://127.0.0.1:5001/demo-tmt-office/us-central1/officePairing'
  );
  for (const value of [
    'http://pair.example/',
    'https://user:secret@pair.example/',
    'https://pair.example/?token=x',
    'https://pair.example/#other',
    'https://pair.example/a/../b',
  ])
    expect(() => pairingEndpoint('cloud', { VITE_OFFICE_PAIRING_URL: value })).toThrow();
});

it('uses the selected owner token only in a no-redirect/no-cookie bounded POST header', async () => {
  const send = vi.fn<typeof fetch>().mockResolvedValue(response());
  const auth = { currentUser: { uid: 'owner', getIdToken: async () => 'private-token' } };
  await createPairingPort(auth, endpoint, send).revoke(revokeRequest, 'owner');
  expect(send).toHaveBeenCalledOnce();
  const [url, options] = send.mock.calls[0];
  expect(url).toBe(`${endpoint}/revoke`);
  expect(options).toMatchObject({
    method: 'POST',
    redirect: 'error',
    credentials: 'omit',
    cache: 'no-store',
    headers: { authorization: 'Bearer private-token' },
  });
  expect(JSON.parse(options?.body as string)).toEqual({
    version: 1,
    publicApproval: revokeRequest,
  });
  expect(options?.signal).toBeInstanceOf(AbortSignal);
});

it('session change during token acquisition cannot send an approval as the previous owner', async () => {
  let resolve: (token: string) => void = () => {};
  const auth: { currentUser: { uid: string; getIdToken(): Promise<string> } | null } = {
    currentUser: {
      uid: 'owner',
      getIdToken: () =>
        new Promise((done) => {
          resolve = done;
        }),
    },
  };
  const send = vi.fn<typeof fetch>();
  const port = createPairingPort(auth, endpoint, send);
  const pending = port.revoke(revokeRequest, 'owner');
  auth.currentUser = null;
  resolve('private-token');
  await expect(pending).rejects.toMatchObject({ kind: 'denied' });
  expect(send).not.toHaveBeenCalled();
  await expect(port.revoke(revokeRequest, 'other')).rejects.toMatchObject({ kind: 'denied' });
});

it('oversize or invalid successful responses remain uncertain and error bodies are never displayed', async () => {
  const auth = { currentUser: { uid: 'owner', getIdToken: async () => 'private-token' } };
  for (const reply of [
    new Response('x'.repeat(4097), { headers: { 'content-type': 'application/json' } }),
    new Response('private server path', { headers: { 'content-type': 'text/plain' } }),
    new Response('private server path', { status: 503 }),
  ]) {
    const port = createPairingPort(auth, endpoint, vi.fn<typeof fetch>().mockResolvedValue(reply));
    await expect(port.revoke(revokeRequest, 'owner')).rejects.toMatchObject({ kind: 'uncertain' });
  }
  const denied = createPairingPort(
    auth,
    endpoint,
    vi.fn<typeof fetch>().mockResolvedValue(new Response('private server path', { status: 403 }))
  );
  await expect(denied.revoke(revokeRequest, 'owner')).rejects.toMatchObject({ kind: 'denied' });
});

it.each([
  [401, 'denied'],
  [403, 'denied'],
  [404, 'unavailable'],
  [409, 'conflict'],
] as const)(
  'maps status %s to the distinct %s recovery state without disclosing the body',
  async (status, kind) => {
    const auth = { currentUser: { uid: 'owner', getIdToken: async () => 'private-token' } };
    const port = createPairingPort(
      auth,
      endpoint,
      vi.fn<typeof fetch>().mockResolvedValue(new Response('sensitive server details', { status }))
    );
    await expect(port.revoke(revokeRequest, 'owner')).rejects.toMatchObject({ kind });
  }
);
