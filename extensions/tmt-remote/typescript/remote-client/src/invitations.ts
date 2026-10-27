import type { Session } from './device.js';
import { verifiedSessionRequest } from './operations.js';
import { base64urlBytes } from './canonical-bytes.js';
import { managementMutation } from './management-mutation.js';
import { parseManagementOutcome, type ManagementOutcome } from './management.js';
import { exact, input, integer, object, valid } from './management-codec.js';

export interface InvitationCandidate {
  clientId: string;
  name: string;
  words: [string, string, string, string];
  snapshot: string;
}
export type PendingInvitation =
  | { state: 'none' }
  | {
      state: 'current';
      offerId: string;
      expiresAtMs: number;
      phase:
        | 'open'
        | 'awaiting-owner'
        | 'confirmed'
        | 'refused'
        | 'cancelled'
        | 'expired'
        | 'unconfirmed';
      localConfirmationKnown: boolean;
      cleanup: 'not-requested' | 'acknowledged' | 'unconfirmed';
      candidate: InvitationCandidate | null;
    };
export interface InvitationDecision {
  operationId: string;
  offerId: string;
  snapshot: string;
  matched: boolean;
  sending?: boolean;
}
export type InvitationOutcome =
  | {
      operationId: string;
      state: 'committed';
      sessionEnded: false;
      result: {
        invitation:
          | { offerId: string; clientId: string; state: 'refused' }
          | {
              offerId: string;
              clientId: string;
              memberId: string;
              state: 'confirmed';
              sendingEnabled: boolean;
            };
      };
    }
  | Exclude<ManagementOutcome, { state: 'committed' }>;
export interface RemoteInvitations {
  pending(): Promise<PendingInvitation>;
  decide(input: InvitationDecision): Promise<InvitationOutcome>;
  operation(operationId: string): Promise<InvitationOutcome>;
}
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const id = (value: unknown): value is string => typeof value === 'string' && UUID.test(value);
const snapshot = (value: unknown): value is string => {
  if (typeof value !== 'string') return false;
  try {
    base64urlBytes(value, 32);
    return true;
  } catch {
    return false;
  }
};
export function parsePendingInvitation(value: unknown): PendingInvitation {
  const row = object(value);
  if (row.state === 'none') exact(row, ['state']);
  else {
    exact(row, [
      'state',
      'offerId',
      'expiresAtMs',
      'phase',
      'localConfirmationKnown',
      'cleanup',
      'candidate',
    ]);
    valid(
      row.state === 'current' &&
        id(row.offerId) &&
        integer(row.expiresAtMs) &&
        typeof row.localConfirmationKnown === 'boolean',
    );
    valid(
      [
        'open',
        'awaiting-owner',
        'confirmed',
        'refused',
        'cancelled',
        'expired',
        'unconfirmed',
      ].includes(row.phase as string),
    );
    valid(['not-requested', 'acknowledged', 'unconfirmed'].includes(row.cleanup as string));
    if (row.candidate !== null) {
      const candidate = object(row.candidate);
      exact(candidate, ['clientId', 'name', 'words', 'snapshot']);
      valid(
        row.phase === 'awaiting-owner' && id(candidate.clientId) && snapshot(candidate.snapshot),
      );
      valid(
        typeof candidate.name === 'string' &&
          candidate.name.length > 0 &&
          candidate.name.length <= 256 &&
          Array.from(candidate.name).every(
            (character) => character.charCodeAt(0) >= 32 && character.charCodeAt(0) !== 127,
          ),
      );
      valid(
        Array.isArray(candidate.words) &&
          candidate.words.length === 4 &&
          candidate.words.every(
            (word) => typeof word === 'string' && /^[a-z]+$/.test(word) && word.length <= 64,
          ),
      );
    }
  }
  return row as unknown as PendingInvitation;
}
export function parseInvitationOutcome(value: unknown, operationId: string): InvitationOutcome {
  const row = object(value);
  if (row.state !== 'committed')
    return parseManagementOutcome(value, operationId) as InvitationOutcome;
  exact(row, ['operationId', 'state', 'sessionEnded', 'result']);
  valid(row.operationId === operationId && row.sessionEnded === false);
  const result = object(row.result);
  exact(result, ['invitation']);
  const invitation = object(result.invitation);
  valid(id(invitation.offerId) && id(invitation.clientId));
  if (invitation.state === 'refused') exact(invitation, ['offerId', 'clientId', 'state']);
  else {
    exact(invitation, ['offerId', 'clientId', 'memberId', 'state', 'sendingEnabled']);
    valid(
      invitation.state === 'confirmed' &&
        typeof invitation.memberId === 'string' &&
        invitation.memberId.length > 0 &&
        invitation.memberId.length <= 128 &&
        typeof invitation.sendingEnabled === 'boolean',
    );
  }
  return row as unknown as InvitationOutcome;
}
/** A presenter over the current offer. No admission, provider call or retry is created here. */
export function invitations(
  session: Session,
  options: { timeoutMs?: number } = {},
): RemoteInvitations {
  const call = verifiedSessionRequest(session, options);
  return {
    pending: () => call('remote.invites.pending', crypto.randomUUID(), {}, parsePendingInvitation),
    decide: (value) => {
      input(
        Object.keys(value).every((key) =>
          ['operationId', 'offerId', 'snapshot', 'matched', 'sending'].includes(key),
        ),
      );
      const frozen = { ...value, sending: value.sending ?? false };
      input(
        id(frozen.operationId) &&
          id(frozen.offerId) &&
          frozen.operationId !== frozen.offerId &&
          snapshot(frozen.snapshot),
      );
      input(
        typeof frozen.matched === 'boolean' &&
          typeof frozen.sending === 'boolean' &&
          (frozen.matched || !frozen.sending),
      );
      return managementMutation(call, 'remote.invites.confirm', frozen, (value) => {
        const outcome = parseInvitationOutcome(value, frozen.operationId);
        if (outcome.state === 'committed') {
          const invitation = outcome.result.invitation;
          valid(
            invitation.offerId === frozen.offerId &&
              invitation.state === (frozen.matched ? 'confirmed' : 'refused'),
          );
          if (invitation.state === 'confirmed') valid(invitation.sendingEnabled === frozen.sending);
        }
        return outcome;
      });
    },
    operation: (operationId) => {
      input(id(operationId));
      return call('remote.management.operation', crypto.randomUUID(), { operationId }, (value) =>
        parseInvitationOutcome(value, operationId),
      );
    },
  };
}
