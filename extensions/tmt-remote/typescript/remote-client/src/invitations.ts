import { invitationDetails, invitationFromInput } from './invite-page.js';
import { channelFor } from './session-channel.js';
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
export interface IssuedInvitation {
  offerId: string;
  issuedAtMs: number;
  expiresAtMs: number;
  state:
    | 'open'
    | 'awaiting-owner'
    | 'confirmed'
    | 'refused'
    | 'cancelled'
    | 'expired'
    | 'unconfirmed'
    | 'interrupted';
  localConfirmationKnown: boolean;
  cleanup: 'not-requested' | 'acknowledged' | 'unconfirmed';
}
export interface IssuedInvitationPage {
  remoteId: string;
  invitations: IssuedInvitation[];
  nextCursor: string | null;
}
export type InvitationIssueOutcome =
  | {
      operationId: string;
      state: 'committed';
      sessionEnded: false;
      result: {
        invitation:
          | { offerId: string; state: 'created'; expiresAtMs: number }
          | {
              offerId: string;
              state: 'cancelled' | 'not-active';
              code: 'REMOTE_INVITATION_CANCELLED' | 'REMOTE_INVITATION_NOT_ACTIVE';
            };
      };
    }
  | Exclude<ManagementOutcome, { state: 'committed' }>;
export type InvitationCreationOutcome =
  | {
      operationId: string;
      state: 'committed';
      sessionEnded: false;
      result: { invitation: { offerId: string; state: 'created'; expiresAtMs: number } };
      secret?: { code: string; link: string | null };
    }
  | Exclude<InvitationIssueOutcome, { state: 'committed' }>;
export type InvitationOriginalOutcome = InvitationOutcome | InvitationIssueOutcome;
export interface RemoteInvitations {
  create(input: { operationId: string }): Promise<InvitationCreationOutcome>;
  list(input: { cursor: string | null }): Promise<IssuedInvitationPage>;
  cancel(input: { operationId: string; offerId: string }): Promise<InvitationIssueOutcome>;
  pending(): Promise<PendingInvitation>;
  decide(input: InvitationDecision): Promise<InvitationOutcome>;
  operation(operationId: string): Promise<InvitationOriginalOutcome>;
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
export function parseIssuedInvitationPage(value: unknown, remoteId: string): IssuedInvitationPage {
  const page = object(value);
  exact(page, ['remoteId', 'invitations', 'nextCursor']);
  valid(
    page.remoteId === remoteId && Array.isArray(page.invitations) && page.invitations.length <= 50,
  );
  valid(
    page.nextCursor === null ||
      (typeof page.nextCursor === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(page.nextCursor)),
  );
  const ids = new Set<string>();
  for (const value of page.invitations as unknown[]) {
    const row = object(value);
    exact(row, [
      'offerId',
      'issuedAtMs',
      'expiresAtMs',
      'state',
      'localConfirmationKnown',
      'cleanup',
    ]);
    valid(id(row.offerId) && !ids.has(row.offerId));
    ids.add(row.offerId);
    valid(
      integer(row.issuedAtMs) &&
        integer(row.expiresAtMs) &&
        (row.expiresAtMs as number) > (row.issuedAtMs as number) &&
        (row.expiresAtMs as number) - (row.issuedAtMs as number) <= 600_000,
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
        'interrupted',
      ].includes(row.state as string),
    );
    valid(
      typeof row.localConfirmationKnown === 'boolean' &&
        ['not-requested', 'acknowledged', 'unconfirmed'].includes(row.cleanup as string),
    );
  }
  return page as unknown as IssuedInvitationPage;
}
export function parseInvitationIssueOutcome(
  value: unknown,
  operationId: string,
  creatingRemote: string,
): InvitationCreationOutcome;
export function parseInvitationIssueOutcome(
  value: unknown,
  operationId: string,
): InvitationIssueOutcome;
export function parseInvitationIssueOutcome(
  value: unknown,
  operationId: string,
  creatingRemote?: string,
): InvitationCreationOutcome | InvitationIssueOutcome {
  const row = object(value);
  if (row.state !== 'committed')
    return parseManagementOutcome(value, operationId) as InvitationIssueOutcome;
  const secret = Object.hasOwn(row, 'secret');
  exact(row, ['operationId', 'state', 'sessionEnded', 'result', ...(secret ? ['secret'] : [])]);
  valid(row.operationId === operationId && row.sessionEnded === false);
  const result = object(row.result);
  exact(result, ['invitation']);
  const invite = object(result.invitation);
  valid(id(invite.offerId));
  if (creatingRemote !== undefined) valid(invite.state === 'created');
  if (invite.state === 'created') {
    exact(invite, ['offerId', 'state', 'expiresAtMs']);
    valid(invite.offerId !== operationId && integer(invite.expiresAtMs));
  } else {
    exact(invite, ['offerId', 'state', 'code']);
    valid(
      (invite.state === 'cancelled' && invite.code === 'REMOTE_INVITATION_CANCELLED') ||
        (invite.state === 'not-active' && invite.code === 'REMOTE_INVITATION_NOT_ACTIVE'),
    );
  }
  if (secret) {
    valid(creatingRemote !== undefined && invite.state === 'created');
    const part = object(row.secret);
    exact(part, ['code', 'link']);
    valid(typeof part.code === 'string' && (part.link === null || typeof part.link === 'string'));
    const details = invitationDetails(part.code as string);
    valid(
      details.owner === creatingRemote &&
        details.offer === invite.offerId &&
        details.expiry === invite.expiresAtMs,
    );
    if (part.link !== null) {
      const linked = invitationFromInput(part.link as string);
      valid(linked.token === part.code);
    }
  }
  return row as unknown as InvitationCreationOutcome | InvitationIssueOutcome;
}
function parseOriginal(value: unknown, id: string): InvitationOriginalOutcome {
  const row = object(value);
  if (
    row.state === 'committed' &&
    ['created', 'cancelled', 'not-active'].includes(
      object(object(row.result).invitation).state as string,
    )
  )
    return parseInvitationIssueOutcome(value, id);
  return parseInvitationOutcome(value, id);
}
/** A presenter over the current offer. No admission, provider call or retry is created here. */
export function invitations(
  session: Session,
  options: { timeoutMs?: number } = {},
): RemoteInvitations {
  const call = verifiedSessionRequest(session, options);
  const remoteId = channelFor(session)?.paired.machineId;
  valid(typeof remoteId === 'string');
  return {
    create: (value) => {
      input(
        Object.keys(value).length === 1 &&
          Object.hasOwn(value, 'operationId') &&
          id(value.operationId),
      );
      const frozen = { ...value };
      return managementMutation(call, 'remote.invites.create', frozen, (reply) =>
        parseInvitationIssueOutcome(reply, frozen.operationId, remoteId),
      );
    },
    list: (value) => {
      input(
        Object.keys(value).length === 1 &&
          Object.hasOwn(value, 'cursor') &&
          (value.cursor === null ||
            (typeof value.cursor === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value.cursor))),
      );
      return call('remote.invites.list', crypto.randomUUID(), { ...value }, (reply) =>
        parseIssuedInvitationPage(reply, remoteId),
      );
    },
    cancel: (value) => {
      input(
        Object.keys(value).length === 2 &&
          Object.hasOwn(value, 'operationId') &&
          Object.hasOwn(value, 'offerId') &&
          id(value.operationId) &&
          id(value.offerId),
      );
      const frozen = { ...value };
      return managementMutation(call, 'remote.invites.cancel', frozen, (reply) => {
        const outcome = parseInvitationIssueOutcome(reply, frozen.operationId);
        if (outcome.state === 'committed')
          valid(
            outcome.result.invitation.offerId === frozen.offerId &&
              outcome.result.invitation.state !== 'created',
          );
        return outcome;
      });
    },
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
        parseOriginal(value, operationId),
      );
    },
  };
}
