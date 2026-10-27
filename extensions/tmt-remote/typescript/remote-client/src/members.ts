import type { Session } from './device.js';
import { channelFor } from './session-channel.js';
import { verifiedSessionRequest } from './operations.js';
import { parseManagementOutcome, type ManagementOutcome } from './management.js';
import { managementMutation } from './management-mutation.js';
import { exact, input, integer, memberName, object, valid } from './management-codec.js';

export interface RemoteMember {
  remoteId: string;
  memberId: string;
  role: 'owner' | 'user';
  displayName: string;
  sendingEnabled: boolean;
  revision: number;
  createdAtMs: number;
  revokedAtMs: number | null;
}
export type MemberInvitationView =
  | { state: 'unavailable' | 'no-current-offer' }
  | {
      state: 'current';
      expiresAtMs: number | null;
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
    };
export type MemberRouteView =
  | { state: 'unavailable' | 'not-recorded' }
  | { state: 'recorded'; publicationPending: boolean; removalPending: boolean };
export interface MemberRoster {
  remoteId: string;
  administrator: boolean;
  displayName: string | null;
  members: RemoteMember[];
  nextCursor: string | null;
  invitation: MemberInvitationView | null;
  routes: MemberRouteView;
}
export interface MemberMachine {
  clientId: string;
  name: string;
  kind: 'browser' | 'addon' | 'cli';
  revision: number;
  revoked: boolean;
  expiresAtMs: number | null;
  sendingScope: boolean;
  delivery: 'direct' | 'hold';
  recipientAccess: 'all' | 'restricted';
  grantLive: boolean;
  lastUsedAtMs: number | null;
  thisBrowser: boolean;
  liveSessionCount: number | null;
  lastActivityAtMs: number | null;
}
export interface MemberMachines {
  remoteId: string;
  memberId: string;
  machines: MemberMachine[];
  nextCursor: string | null;
}
export interface MemberPage {
  cursor: string | null;
  limit: number;
}
export interface MemberChange {
  operationId: string;
  remoteId: string;
  memberId: string;
  memberRevision: number;
}
export type MemberOutcome =
  | {
      operationId: string;
      state: 'committed';
      result: { member: RemoteMember } | { remote: { remoteId: string; displayName: string } };
      sessionEnded: boolean;
    }
  | Exclude<ManagementOutcome, { state: 'committed' }>;
export type MemberOriginalOutcome = MemberOutcome | ManagementOutcome;
export interface RemoteMembers {
  readonly clientId: string;
  list(page: MemberPage): Promise<MemberRoster>;
  machines(page: MemberPage & { memberId: string }): Promise<MemberMachines>;
  rename(change: MemberChange & { name: string }): Promise<MemberOutcome>;
  sending(change: MemberChange & { enabled: boolean }): Promise<MemberOutcome>;
  revoke(change: MemberChange): Promise<MemberOutcome>;
  renameRemote(change: {
    operationId: string;
    remoteId: string;
    name: string;
  }): Promise<MemberOutcome>;
  operation(operationId: string): Promise<MemberOriginalOutcome>;
}
const V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const cursor = (v: unknown) =>
  v === null || (typeof v === 'string' && /^[A-Za-z0-9_-]{54}$/.test(v));
const id = (v: unknown): v is string => typeof v === 'string' && v.length > 0;
const revision = (v: unknown) => integer(v) && (v as number) > 0;
const time = (v: unknown) => v === null || integer(v);
function inputKeys(value: object, keys: string[]) {
  input(Object.keys(value).length === keys.length && keys.every((k) => Object.hasOwn(value, k)));
}
function page(value: MemberPage) {
  input(cursor(value.cursor) && integer(value.limit) && value.limit >= 1 && value.limit <= 50);
}
function person(value: unknown): RemoteMember {
  const row = object(value);
  exact(row, [
    'remoteId',
    'memberId',
    'role',
    'displayName',
    'sendingEnabled',
    'revision',
    'createdAtMs',
    'revokedAtMs',
  ]);
  valid(
    typeof row.remoteId === 'string' &&
      V4.test(row.remoteId) &&
      id(row.memberId) &&
      ['owner', 'user'].includes(row.role as string) &&
      memberName(row.displayName) &&
      typeof row.sendingEnabled === 'boolean' &&
      revision(row.revision) &&
      integer(row.createdAtMs) &&
      time(row.revokedAtMs),
  );
  return row as unknown as RemoteMember;
}
function invitation(value: unknown): MemberInvitationView {
  const row = object(value);
  if (row.state === 'unavailable' || row.state === 'no-current-offer') exact(row, ['state']);
  else {
    exact(row, ['state', 'expiresAtMs', 'phase', 'localConfirmationKnown', 'cleanup']);
    valid(
      row.state === 'current' &&
        time(row.expiresAtMs) &&
        [
          'open',
          'awaiting-owner',
          'confirmed',
          'refused',
          'cancelled',
          'expired',
          'unconfirmed',
        ].includes(row.phase as string) &&
        typeof row.localConfirmationKnown === 'boolean' &&
        ['not-requested', 'acknowledged', 'unconfirmed'].includes(row.cleanup as string),
    );
  }
  return row as unknown as MemberInvitationView;
}
function routes(value: unknown): MemberRouteView {
  const row = object(value);
  if (row.state === 'unavailable' || row.state === 'not-recorded') exact(row, ['state']);
  else {
    exact(row, ['state', 'publicationPending', 'removalPending']);
    valid(
      row.state === 'recorded' &&
        typeof row.publicationPending === 'boolean' &&
        typeof row.removalPending === 'boolean',
    );
  }
  return row as unknown as MemberRouteView;
}
function machine(value: unknown, caller: string): MemberMachine {
  const row = object(value);
  exact(row, [
    'clientId',
    'name',
    'kind',
    'revision',
    'revoked',
    'expiresAtMs',
    'sendingScope',
    'delivery',
    'recipientAccess',
    'grantLive',
    'lastUsedAtMs',
    'thisBrowser',
    'liveSessionCount',
    'lastActivityAtMs',
  ]);
  valid(
    id(row.clientId) &&
      memberName(row.name) &&
      ['browser', 'addon', 'cli'].includes(row.kind as string) &&
      revision(row.revision) &&
      typeof row.revoked === 'boolean' &&
      time(row.expiresAtMs) &&
      typeof row.sendingScope === 'boolean' &&
      ['direct', 'hold'].includes(row.delivery as string) &&
      ['all', 'restricted'].includes(row.recipientAccess as string) &&
      typeof row.grantLive === 'boolean' &&
      time(row.lastUsedAtMs) &&
      row.thisBrowser === (row.clientId === caller) &&
      time(row.liveSessionCount) &&
      time(row.lastActivityAtMs),
  );
  valid(!row.revoked || !row.grantLive);
  valid(
    row.liveSessionCount === null || row.liveSessionCount === 0
      ? row.lastActivityAtMs === null
      : integer(row.lastActivityAtMs),
  );
  return row as unknown as MemberMachine;
}
function original(value: unknown, operationId: string): MemberOriginalOutcome {
  const row = object(value);
  if (row.state !== 'committed') return parseManagementOutcome(value, operationId);
  const result = object(row.result);
  if (!Object.hasOwn(result, 'member') && !Object.hasOwn(result, 'remote'))
    return parseManagementOutcome(value, operationId);
  exact(row, ['operationId', 'state', 'result', 'sessionEnded']);
  valid(row.operationId === operationId && typeof row.sessionEnded === 'boolean');
  if (Object.hasOwn(result, 'member')) {
    exact(result, ['member']);
    person(result.member);
  } else {
    exact(result, ['remote']);
    const remote = object(result.remote);
    exact(remote, ['remoteId', 'displayName']);
    valid(
      typeof remote.remoteId === 'string' &&
        V4.test(remote.remoteId) &&
        memberName(remote.displayName) &&
        row.sessionEnded === false,
    );
  }
  return row as unknown as MemberOutcome;
}

/** Availability and observations grant no authority. Unknown mutations retain their original ID. */
export function members(session: Session, options: { timeoutMs?: number } = {}): RemoteMembers {
  const call = verifiedSessionRequest(session, options);
  const paired = channelFor(session)!.paired;
  const clientId = paired.clientId;
  const remoteId = paired.machineId;
  async function mutate(
    operation: string,
    change: MemberChange | { operationId: string; remoteId: string; name: string },
    kind: 'member' | 'remote',
  ): Promise<MemberOutcome> {
    const frozen = { ...change };
    input(V4.test(frozen.operationId) && V4.test(frozen.remoteId) && frozen.remoteId === remoteId);
    return managementMutation(call, operation, frozen, (value) => {
      const parsed = original(value, frozen.operationId);
      if (parsed.state === 'committed') {
        if (kind === 'remote') {
          valid(
            'remote' in parsed.result &&
              parsed.result.remote.remoteId === frozen.remoteId &&
              parsed.result.remote.displayName === (frozen as { name: string }).name,
          );
        } else {
          valid(
            'member' in parsed.result &&
              'memberId' in frozen &&
              parsed.result.member.remoteId === frozen.remoteId &&
              parsed.result.member.memberId === frozen.memberId &&
              parsed.result.member.revision >= frozen.memberRevision,
          );
          if (operation === 'remote.members.rename')
            valid(
              parsed.result.member.displayName ===
                (frozen as MemberChange & { name: string }).name && parsed.sessionEnded === false,
            );
          if (operation === 'remote.members.sending')
            valid(
              parsed.result.member.sendingEnabled ===
                (frozen as MemberChange & { enabled: boolean }).enabled,
            );
          if (operation === 'remote.members.revoke')
            valid(
              parsed.result.member.revokedAtMs !== null && !parsed.result.member.sendingEnabled,
            );
        }
      }
      return parsed as MemberOutcome;
    });
  }
  function target(change: MemberChange) {
    input(id(change.memberId) && revision(change.memberRevision));
  }
  return {
    clientId,
    list: (value) => {
      inputKeys(value, ['cursor', 'limit']);
      page(value);
      const frozen = { ...value };
      return call('remote.members.list', crypto.randomUUID(), frozen, (value) => {
        const row = object(value);
        exact(row, [
          'remoteId',
          'administrator',
          'displayName',
          'members',
          'nextCursor',
          'invitation',
          'routes',
        ]);
        valid(
          row.remoteId === remoteId &&
            typeof row.administrator === 'boolean' &&
            (row.displayName === null || memberName(row.displayName)) &&
            Array.isArray(row.members) &&
            row.members.length <= frozen.limit &&
            cursor(row.nextCursor),
        );
        const seen = new Set<string>();
        for (const entry of row.members) {
          const member = person(entry);
          valid(member.remoteId === row.remoteId && !seen.has(member.memberId));
          seen.add(member.memberId);
        }
        if (row.administrator) invitation(row.invitation);
        else valid(row.members.length <= 1 && row.invitation === null);
        routes(row.routes);
        return row as unknown as MemberRoster;
      });
    },
    machines: (value) => {
      inputKeys(value, ['memberId', 'cursor', 'limit']);
      page(value);
      input(id(value.memberId));
      const frozen = { ...value };
      return call('remote.members.machines', crypto.randomUUID(), frozen, (value) => {
        const row = object(value);
        exact(row, ['remoteId', 'memberId', 'machines', 'nextCursor']);
        valid(
          row.remoteId === remoteId &&
            row.memberId === frozen.memberId &&
            Array.isArray(row.machines) &&
            row.machines.length <= frozen.limit &&
            cursor(row.nextCursor),
        );
        const seen = new Set<string>();
        for (const entry of row.machines) {
          const item = machine(entry, clientId);
          valid(!seen.has(item.clientId));
          seen.add(item.clientId);
        }
        return row as unknown as MemberMachines;
      });
    },
    rename: (change) => {
      inputKeys(change, ['operationId', 'remoteId', 'memberId', 'memberRevision', 'name']);
      target(change);
      input(memberName(change.name));
      return mutate('remote.members.rename', { ...change }, 'member');
    },
    sending: (change) => {
      inputKeys(change, ['operationId', 'remoteId', 'memberId', 'memberRevision', 'enabled']);
      target(change);
      input(typeof change.enabled === 'boolean');
      return mutate('remote.members.sending', { ...change }, 'member');
    },
    revoke: (change) => {
      inputKeys(change, ['operationId', 'remoteId', 'memberId', 'memberRevision']);
      target(change);
      return mutate('remote.members.revoke', { ...change }, 'member');
    },
    renameRemote: (change) => {
      inputKeys(change, ['operationId', 'remoteId', 'name']);
      input(memberName(change.name));
      return mutate('remote.name.set', { ...change }, 'remote');
    },
    operation: (operationId) => {
      input(V4.test(operationId));
      return call('remote.management.operation', crypto.randomUUID(), { operationId }, (value) => {
        const parsed = original(value, operationId);
        if (parsed.state === 'committed') {
          if ('member' in parsed.result) valid(parsed.result.member.remoteId === remoteId);
          if ('remote' in parsed.result) valid(parsed.result.remote.remoteId === remoteId);
        }
        return parsed;
      });
    },
  };
}
