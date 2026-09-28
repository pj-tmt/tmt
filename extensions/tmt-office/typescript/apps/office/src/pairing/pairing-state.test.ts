import { expect, it, vi } from 'vitest';
import { createPairingState } from './pairing-state.js';
import { PairingActionError } from './pairing-contract.js';
import type { ApprovedPairing, PairingRequest, PairingPort } from './pairing-contract.js';

const request: PairingRequest = {
  version: 1,
  pairingId: 'a'.repeat(64),
  worldId: 'a'.repeat(20),
  installationId: '00000000-0000-4000-8000-000000000001',
  identityId: '00000000-0000-4000-8000-000000000002',
  installationLabel: 'Workstation',
  identityLabel: 'Alice',
  capabilities: ['layout.read'],
};
const binding: ApprovedPairing = {
  ...request,
  principalUid: 'office-agent:00000000-0000-4000-8000-000000000003',
  blockId: '00000000-0000-4000-8000-000000000004',
  expiresAt: 2_000_000_000_000,
};
function fixture() {
  const port: PairingPort = { approve: vi.fn(async () => binding), revoke: vi.fn(async () => {}) };
  return { port, state: createPairingState(port, request, 'owner') };
}

it('freezes a copied assignment choice before uncertain approval and keeps it through retry', async () => {
  const { port, state } = fixture();
  const selection = { principalUid: binding.principalUid, blockId: binding.blockId };
  state.selectReplacement(selection);
  selection.blockId = request.identityId;
  vi.mocked(port.approve).mockRejectedValueOnce(new PairingActionError('uncertain'));
  await state.approve();
  state.selectReplacement(null);
  await state.approve();
  expect(port.approve).toHaveBeenCalledTimes(2);
  for (const call of vi.mocked(port.approve).mock.calls)
    expect(call).toEqual([
      request,
      'owner',
      { principalUid: binding.principalUid, blockId: binding.blockId },
    ]);
  state.dispose();
  state.selectReplacement(selection);
  expect(state.getSnapshot().replacement).toBeNull();
});

it('viewing is read-only; concurrent actions serialize and uncertain approval retries preserve the request', async () => {
  const { port, state } = fixture();
  expect(port.approve).not.toHaveBeenCalled();
  let reject: (error: Error) => void = () => {};
  vi.mocked(port.approve).mockImplementationOnce(
    () =>
      new Promise((_, fail) => {
        reject = fail;
      })
  );
  const pending = state.approve();
  await state.approve();
  await state.revoke();
  expect(port.approve).toHaveBeenCalledExactlyOnceWith(request, 'owner');
  expect(port.revoke).not.toHaveBeenCalled();
  reject(new Error('Do not expose a bearer or raw service failure'));
  await pending;
  expect(state.getSnapshot()).toMatchObject({ attempted: true, busy: false, approved: null });
  expect(state.getSnapshot().error).toContain('may already have completed');
  expect(state.getSnapshot().error).not.toContain('bearer');
  await state.approve();
  expect(port.approve).toHaveBeenLastCalledWith(request, 'owner');
  expect(state.getSnapshot().approved).toEqual(binding);
  state.dispose();
});

it('failed revocation preserves confirmed approval; explicit retry revokes once without reapproval', async () => {
  const { port, state } = fixture();
  await state.approve();
  vi.mocked(port.revoke).mockRejectedValueOnce(new PairingActionError('uncertain'));
  await state.revoke();
  expect(state.getSnapshot()).toMatchObject({ approved: binding, revoked: false });
  await state.revoke();
  expect(port.revoke).toHaveBeenLastCalledWith(request, 'owner');
  expect(state.getSnapshot()).toMatchObject({ approved: null, revoked: true, error: null });
  await state.approve();
  expect(port.approve).toHaveBeenCalledOnce();
  state.dispose();
});

it('cancels before approval and retries the same immutable request after uncertainty', async () => {
  const { port, state } = fixture();
  vi.mocked(port.revoke).mockRejectedValueOnce(new PairingActionError('uncertain'));
  await state.revoke();
  expect(state.getSnapshot()).toMatchObject({
    approved: null,
    revoked: false,
    revocationAttempted: true,
  });
  state.selectReplacement({ principalUid: binding.principalUid, blockId: binding.blockId });
  expect(state.getSnapshot().replacement).toBeNull();
  await state.approve();
  expect(port.approve).not.toHaveBeenCalled();
  await state.revoke();
  expect(port.revoke).toHaveBeenNthCalledWith(1, request, 'owner');
  expect(port.revoke).toHaveBeenNthCalledWith(2, request, 'owner');
  expect(state.getSnapshot()).toMatchObject({ revoked: true, error: null });
  state.dispose();
});

it('freezes the original request for approval and revocation retries', async () => {
  const { port, state } = fixture();
  await state.approve();
  await state.revoke();
  const approvalRequest = vi.mocked(port.approve).mock.calls[0][0];
  const revokeRequest = vi.mocked(port.revoke).mock.calls[0][0];
  expect(approvalRequest).toBe(revokeRequest);
  expect(() => {
    approvalRequest.identityLabel = 'changed';
  }).toThrow();
  expect(() => {
    (approvalRequest.capabilities as unknown as string[])[0] = 'layout.write';
  }).toThrow();
  state.dispose();
});

it('disposal clears private results, suppresses late completion and prevents subsequent actions', async () => {
  const { port, state } = fixture();
  let resolve: (binding: ApprovedPairing) => void = () => {};
  vi.mocked(port.approve).mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      })
  );
  const changed = vi.fn();
  state.subscribe(changed);
  const pending = state.approve();
  expect(changed).toHaveBeenCalledOnce();
  state.dispose();
  resolve(binding);
  await pending;
  await state.approve();
  expect(changed).toHaveBeenCalledOnce();
  expect(state.getSnapshot()).toMatchObject({ approved: null, attempted: false, busy: false });
  expect(port.approve).toHaveBeenCalledOnce();
});

it('disposal fences a pending cancellation and prevents a later retry', async () => {
  let resolve: () => void = () => {};
  const { port, state } = fixture();
  vi.mocked(port.revoke).mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      })
  );
  const changed = vi.fn();
  state.subscribe(changed);
  const pending = state.revoke();
  expect(changed).toHaveBeenCalledOnce();
  state.dispose();
  resolve();
  await pending;
  await state.revoke();
  expect(changed).toHaveBeenCalledOnce();
  expect(port.revoke).toHaveBeenCalledOnce();
  expect(state.getSnapshot()).toMatchObject({ revoked: false, revocationAttempted: false });
});
