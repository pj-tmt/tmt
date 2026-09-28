import type { DecodedIdToken } from 'firebase-admin/auth';
import {
  PairingError,
  UUID,
  parseOwnerApproval,
  parseClaim,
  parseRenewal,
  parseRevocation,
} from './pairing-contract.js';
import type { AgentIdentity, PairingStore } from './pairing-store.js';

export interface PairingAuthentication {
  verifyIdToken(token: string, checkRevoked: boolean): Promise<DecodedIdToken>;
  createCustomToken(uid: string, claims: Record<string, unknown>): Promise<string>;
}

export function createPairingService(store: PairingStore, auth: PairingAuthentication) {
  async function verified(authorization: string | undefined): Promise<DecodedIdToken> {
    if (!authorization || authorization.length > 8192 || !/^Bearer \S+$/.test(authorization))
      throw new PairingError('UNAUTHENTICATED');
    try {
      return await auth.verifyIdToken(authorization.slice(7), true);
    } catch {
      throw new PairingError('UNAUTHENTICATED');
    }
  }

  function human(token: DecodedIdToken): string {
    if (
      token.tmtOfficeAgent === true ||
      token.firebase?.sign_in_provider !== 'google.com' ||
      token.email_verified !== true
    )
      throw new PairingError('PERMISSION_DENIED');
    return token.uid;
  }

  function agent(token: DecodedIdToken): AgentIdentity {
    if (
      token.tmtOfficeAgent !== true ||
      typeof token.tmtInstallationId !== 'string' ||
      !UUID.test(token.tmtInstallationId) ||
      typeof token.tmtIdentityId !== 'string' ||
      !UUID.test(token.tmtIdentityId)
    )
      throw new PairingError('PERMISSION_DENIED');
    return {
      uid: token.uid,
      installationId: token.tmtInstallationId,
      identityId: token.tmtIdentityId,
    };
  }

  return {
    async approve(input: unknown, authorization?: string) {
      const approval = parseOwnerApproval(input);
      return store.approve(
        human(await verified(authorization)),
        approval.request,
        approval.replacesPrincipalUid
      );
    },
    async claim(input: unknown) {
      const id = parseClaim(input);
      const binding = await store.reserveClaim(id);
      let customToken: string;
      try {
        customToken = await auth.createCustomToken(binding.principalUid, {
          tmtOfficeAgent: true,
          tmtInstallationId: binding.installationId,
          tmtIdentityId: binding.identityId,
        });
      } catch {
        throw new PairingError('UNAVAILABLE');
      }
      await store.confirmClaim(id);
      return { ...binding, customToken };
    },
    async renew(input: unknown, authorization?: string) {
      const renewal = parseRenewal(input);
      const binding = await store.renew(
        agent(await verified(authorization)),
        renewal.pairingId,
        renewal.grantExpiresAt
      );
      return binding;
    },
    async revoke(input: unknown, authorization?: string) {
      const request = parseRevocation(input);
      if (request.kind === 'proof') {
        if (authorization !== undefined) throw new PairingError('INVALID_ARGUMENT');
        await store.revoke({ kind: 'proof' }, request.pairingId);
      } else if (request.kind === 'ownerApproval') {
        await store.revoke(
          {
            kind: 'ownerApproval',
            uid: human(await verified(authorization)),
            request: request.request,
          },
          request.request.pairingId
        );
      } else {
        const token = await verified(authorization);
        await store.revoke(
          token.tmtOfficeAgent === true
            ? { kind: 'agent', agent: agent(token) }
            : { kind: 'owner', uid: human(token) },
          request.pairingId
        );
      }
      return {
        version: 1,
        pairingId: request.kind === 'ownerApproval' ? request.request.pairingId : request.pairingId,
        revoked: true,
      };
    },
  };
}
