import { PairingError } from './pairing-contract.js';
import type { PairingCode } from './pairing-contract.js';
import type { createPairingService } from './pairing-service.js';

export interface PairingRequest {
  method: string;
  path: string;
  authorization?: string;
  origin?: string;
  contentType?: string;
  bodyBytes: number;
  body: unknown;
}

const statuses: Record<PairingCode, number> = {
  INVALID_ARGUMENT: 400,
  UNAUTHENTICATED: 401,
  PERMISSION_DENIED: 403,
  PAIRING_UNAVAILABLE: 404,
  PAIRING_CONFLICT: 409,
  RETRY_LATER: 429,
  UNAVAILABLE: 503,
};

export function createPairingHandler(
  service: ReturnType<typeof createPairingService>,
  allowedOrigin: string | undefined
) {
  return async (request: PairingRequest): Promise<{ status: number; body: unknown }> => {
    const error = (status: number, code: string) => ({ status, body: { error: { code } } });
    if (request.method !== 'POST') return error(405, 'METHOD_NOT_ALLOWED');
    if (request.origin && request.origin !== allowedOrigin) return error(403, 'PERMISSION_DENIED');
    if (
      !Number.isSafeInteger(request.bodyBytes) ||
      request.bodyBytes < 0 ||
      request.bodyBytes > 4096
    )
      return error(413, 'INPUT_TOO_LARGE');
    if (!/^application\/json(?:\s*;|$)/i.test(request.contentType ?? ''))
      return error(400, 'INVALID_ARGUMENT');
    try {
      switch (request.path) {
        case '/approve':
          return { status: 200, body: await service.approve(request.body, request.authorization) };
        case '/claim':
          return { status: 200, body: await service.claim(request.body) };
        case '/renew':
          return { status: 200, body: await service.renew(request.body, request.authorization) };
        case '/revoke':
          return { status: 200, body: await service.revoke(request.body, request.authorization) };
        default:
          return error(404, 'NOT_FOUND');
      }
    } catch (failure) {
      if (failure instanceof PairingError) return error(statuses[failure.code], failure.code);
      return error(503, 'UNAVAILABLE');
    }
  };
}
