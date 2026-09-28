import {
  parseApprovedPairing,
  parseRevokedPairing,
  PairingActionError,
  snapshotPairingRequest,
  validatePairingReplacement,
} from './pairing-contract.js';
import type { PairingPort } from './pairing-contract.js';
import { readBoundedBody } from '../transport/response-body.js';

async function responseBody(response: Response): Promise<unknown> {
  if (
    !response.headers.get('content-type')?.match(/^application\/json(?:\s*;|$)/i) ||
    !response.body
  )
    throw new PairingActionError('uncertain');
  return JSON.parse(
    new TextDecoder('utf-8', { fatal: true }).decode(await readBoundedBody(response, 4096))
  );
}

interface TokenSession {
  readonly currentUser: { uid: string; getIdToken(): Promise<string> } | null;
}

export function createPairingPort(
  auth: TokenSession,
  endpoint: string,
  send: typeof fetch = fetch
): PairingPort {
  async function post(operation: 'approve' | 'revoke', input: unknown, ownerUid: string) {
    const user = auth.currentUser;
    if (!user || user.uid !== ownerUid) throw new PairingActionError('denied');
    try {
      const token = await user.getIdToken();
      if (auth.currentUser !== user) throw new PairingActionError('denied');
      const response = await send(`${endpoint}/${operation}`, {
        method: 'POST',
        headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
        body: JSON.stringify(input),
        credentials: 'omit',
        redirect: 'error',
        cache: 'no-store',
        signal: AbortSignal.timeout(10_000),
      });
      const failure =
        auth.currentUser !== user || [400, 401, 403].includes(response.status)
          ? 'denied'
          : response.status === 404
            ? 'unavailable'
            : response.status === 409
              ? 'conflict'
              : response.status !== 200
                ? 'uncertain'
                : undefined;
      if (failure) {
        await response.body?.cancel();
        throw new PairingActionError(failure);
      }
      return await responseBody(response);
    } catch (error) {
      if (error instanceof PairingActionError) throw error;
      throw new PairingActionError('uncertain');
    }
  }
  return {
    async approve(request, ownerUid, replacement) {
      const originalRequest = snapshotPairingRequest(request);
      if (replacement) validatePairingReplacement(replacement);
      const input = replacement
        ? { ...originalRequest, replacesPrincipalUid: replacement.principalUid }
        : originalRequest;
      const approved = parseApprovedPairing(
        await post('approve', input, ownerUid),
        originalRequest
      );
      if (
        replacement &&
        (approved.blockId !== replacement.blockId ||
          approved.principalUid === replacement.principalUid)
      )
        throw new PairingActionError('uncertain');
      return approved;
    },
    async revoke(request, ownerUid) {
      const originalRequest = snapshotPairingRequest(request);
      parseRevokedPairing(
        await post('revoke', { version: 1, publicApproval: originalRequest }, ownerUid),
        originalRequest.pairingId
      );
    },
  };
}
