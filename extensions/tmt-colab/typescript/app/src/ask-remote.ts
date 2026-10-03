import { coreId, decimal, exactKeys, generatedId, requireValue, time } from '@tmt/colab-client';
import { requestId } from './ask-records.js';
import { remoteSdk, jsonResponse } from './registration.js';

export interface RemoteAgent {
  id: string;
  name: string;
  presence?: 'active' | 'offline' | 'unknown';
}
export interface SendInput {
  operationId: string;
  agentId: string;
  message: string;
}
export type SendState =
  | { state: 'held'; operationId: string }
  | { state: 'accepted'; operationId: string; requestId: string }
  | { state: 'uncertain'; operationId: string; reason?: SessionEndCode }
  | { state: 'refused' | 'cancelled'; operationId: string; reason?: string };
export const REMOTE_REFUSAL_CODES = [
  'REMOTE_SCOPE_DENIED',
  'REMOTE_INPUT_INVALID',
  'REMOTE_RATE_LIMITED',
  'REMOTE_INTENT_CONFLICT',
  'REMOTE_CLOSED',
  'REMOTE_SESSION_ENDED',
  'REMOTE_INPUT_TOO_LARGE',
  'REMOTE_STATE_UNAVAILABLE',
  'REMOTE_CORE_UNAVAILABLE',
] as const;
export type RemoteRefusalCode = (typeof REMOTE_REFUSAL_CODES)[number];
export type SessionEndCode = 'REMOTE_SESSION_ENDED' | 'REMOTE_SEQUENCE_UNAVAILABLE';
export function sessionEndedReason(reason: string | undefined): reason is SessionEndCode {
  return reason === 'REMOTE_SESSION_ENDED' || reason === 'REMOTE_SEQUENCE_UNAVAILABLE';
}
/** An adapter-normalized session fault, derived only from verified SDK types
 * or a definitive verified SendState; it never authorizes another dispatch. */
export class SessionEndedError extends Error {
  constructor(readonly code: SessionEndCode) {
    super(code);
  }
}
type SdkError = abstract new (...args: never[]) => Error & { code: string };
export function refusalReason(reason: string | undefined): RemoteRefusalCode | 'REMOTE_REFUSED' {
  return reason !== undefined && (REMOTE_REFUSAL_CODES as readonly string[]).includes(reason)
    ? (reason as RemoteRefusalCode)
    : 'REMOTE_REFUSED';
}
/** Verified read refusals are transient observations, never new ledger states. */
export class ReadRefusedError extends Error {
  constructor(readonly code: RemoteRefusalCode | 'REMOTE_REFUSED') {
    super(code);
  }
}
export type ResultState =
  | { state: 'pending'; requestId: string }
  | { state: 'replied'; requestId: string; message: string }
  | { state: 'unavailable'; requestId: string; reason?: string };
export interface RemoteContext {
  machineId: string;
  deviceId: string;
  grantRevision: string;
  deviceName: string;
  expiresAtMs: number | null;
  mode: 'direct' | 'hold' | null;
}
export interface RemoteClient {
  context(): Promise<RemoteContext>;
  listAgents(): Promise<RemoteAgent[]>;
  send(input: SendInput): Promise<SendState>;
  operation(operationId: string): Promise<SendState>;
  result(requestId: string): Promise<ResultState>;
}
/** The served Remote SDK owns credentials, sequence and response verification.
 * Colab consumes its public helper; it never signs raw Remote envelopes. */
export interface OperationsSdk {
  ClientError?: SdkError;
  RefusalError?: SdkError;
  operations(
    session: unknown,
    options?: { timeoutMs?: number },
  ): Pick<RemoteClient, 'send' | 'operation'> & {
    result(
      requestId: string,
    ): Promise<
      | { state: 'pending'; requestId?: string }
      | { state: 'replied'; requestId?: string; message: string }
      | { state: 'unavailable'; requestId?: string; reason?: string }
    >;
    listAgents(): Promise<
      { id: string; name: string; presence: 'active' | 'offline' | 'unknown'; delivery?: unknown }[]
    >;
  };
}
function state(value: SendState, id: string): SendState {
  requireValue(
    value.operationId === id &&
      ['accepted', 'held', 'uncertain', 'refused', 'cancelled'].includes(value.state),
  );
  if (value.state === 'accepted') requestId(value.requestId);
  return structuredClone(value);
}
/** Wrap the exact verified session retained by Registration/Live. Reopening
 * here would end Live's tunnels; the SDK owns sequence resync and same-ID reads.
 * Registration must rebuild this client and its AskControllers when replacing
 * the Session; an old client never adopts a replacement session implicitly. */
export async function createRemoteClient(
  mount: URL,
  supplied?: OperationsSdk,
  opened?: unknown,
  send: typeof fetch = fetch,
): Promise<RemoteClient> {
  requireValue(opened !== null && typeof opened === 'object');
  const session = opened as Record<string, unknown>;
  requireValue(typeof session.sessionId === 'string');
  time(session.serverTimeMs as number);
  const grantRevision = String(session.grantRevision);
  decimal(grantRevision);
  const expiresAtMs = session.expiresAtMs;
  requireValue(expiresAtMs === null || typeof expiresAtMs === 'number');
  if (expiresAtMs !== null) time(expiresAtMs as number);
  const sdk = supplied ?? ((await remoteSdk()) as unknown as OperationsSdk);
  requireValue(typeof sdk.operations === 'function');
  const ops = sdk.operations(opened, { timeoutMs: 20000 });
  const sessionFault = (error: unknown): SessionEndCode | undefined => {
    if (
      sdk.RefusalError &&
      error instanceof sdk.RefusalError &&
      error.code === 'REMOTE_SESSION_ENDED'
    )
      return 'REMOTE_SESSION_ENDED';
    if (
      sdk.ClientError &&
      error instanceof sdk.ClientError &&
      error.code === 'sequence_unavailable'
    )
      return 'REMOTE_SEQUENCE_UNAVAILABLE';
    return undefined;
  };
  const observe = async <T>(action: () => Promise<T>): Promise<T> => {
    try {
      return await action();
    } catch (error) {
      const code = sessionFault(error);
      if (code) throw new SessionEndedError(code);
      if (sdk.RefusalError && error instanceof sdk.RefusalError)
        throw new ReadRefusedError(refusalReason(error.code));
      throw error;
    }
  };
  return {
    context: async () => {
      if (expiresAtMs !== null && Date.now() >= (expiresAtMs as number))
        throw new SessionEndedError('REMOTE_SESSION_ENDED');
      const current = await jsonResponse(
        await send(new URL('api/session', mount), {
          signal: AbortSignal.timeout(10000),
        }),
        8192,
      );
      const door = await jsonResponse(
        await send(new URL('/sdk/mount', mount), {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ path: mount.pathname }),
          signal: AbortSignal.timeout(10000),
        }),
        8192,
      );
      exactKeys(current, ['deviceId', 'publicKey', 'grantRevision', 'name']);
      exactKeys(door, ['machineId', 'windowId', 'address', 'extension', 'mount']);
      generatedId(current.deviceId as string);
      generatedId(door.machineId as string);
      decimal(current.grantRevision as string);
      if (current.grantRevision !== grantRevision)
        throw new SessionEndedError('REMOTE_SESSION_ENDED');
      requireValue(typeof current.name === 'string' && current.name.length > 0);
      requireValue(door.extension === 'colab' && door.mount === mount.pathname);
      return {
        machineId: door.machineId as string,
        deviceId: current.deviceId as string,
        grantRevision,
        deviceName: current.name as string,
        expiresAtMs: expiresAtMs as number | null,
        mode: null,
      };
    },
    listAgents: async () => {
      const rows = await observe(() => ops.listAgents());
      requireValue(Array.isArray(rows) && rows.length <= 256);
      for (const row of rows) {
        coreId(row.id);
        requireValue(
          typeof row.name === 'string' && ['active', 'offline', 'unknown'].includes(row.presence),
        );
      }
      return rows.map(({ id, name, presence }) => ({ id, name, presence }));
    },
    send: async (input) => {
      generatedId(input.operationId);
      coreId(input.agentId);
      try {
        return state(await ops.send(input), input.operationId);
      } catch (error) {
        return { state: 'uncertain', operationId: input.operationId, reason: sessionFault(error) };
      }
    },
    operation: async (id) => {
      generatedId(id);
      try {
        return state(await observe(() => ops.operation(id)), id);
      } catch (error) {
        if (error instanceof SessionEndedError || error instanceof ReadRefusedError) throw error;
        return { state: 'uncertain', operationId: id, reason: sessionFault(error) };
      }
    },
    result: async (id) => {
      requestId(id);
      const result = await observe(() => ops.result(id));
      requireValue(
        (result.requestId === undefined || result.requestId === id) &&
          ['pending', 'replied', 'unavailable'].includes(result.state),
      );
      if (result.state === 'replied') requireValue(typeof result.message === 'string');
      return { ...structuredClone(result), requestId: id } as ResultState;
    },
  };
}
