import { coreId, decimal, exactKeys, generatedId, requireValue, time } from '@tmt/colab-client';
import { requestId } from './ask-records.js';
import { remoteSdk, jsonResponse } from './registration.js';

export type Delivery = 'channel' | 'paste' | 'not_ready' | 'not_running';
export interface RemoteAgent {
  id: string;
  name: string;
  presence?: 'active' | 'offline' | 'unknown';
  delivery?: Delivery;
}
export interface SendInput {
  operationId: string;
  agentId: string;
  message: string;
}
export type SendState =
  | { state: 'held'; operationId: string }
  | { state: 'accepted'; operationId: string; requestId: string }
  | { state: 'uncertain'; operationId: string; requestId?: string }
  | { state: 'refused' | 'cancelled'; operationId: string; reason?: string };
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
  check(agentId: string): Promise<unknown>;
}
/** The served Remote SDK owns credentials, sequence and response verification.
 * Colab consumes its public helper; it never signs raw Remote envelopes. */
export interface OperationsSdk {
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
 * here would end Live's tunnels; the SDK owns sequence resync and same-ID reads. */
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
  return {
    context: async () => {
      requireValue(expiresAtMs === null || Date.now() < (expiresAtMs as number));
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
      requireValue(
        current.grantRevision === grantRevision &&
          typeof current.name === 'string' &&
          current.name.length > 0,
      );
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
      const rows = await ops.listAgents();
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
      } catch {
        return { state: 'uncertain', operationId: input.operationId };
      }
    },
    operation: async (id) => {
      generatedId(id);
      try {
        return state(await ops.operation(id), id);
      } catch {
        return { state: 'uncertain', operationId: id };
      }
    },
    result: async (id) => {
      requestId(id);
      const result = await ops.result(id);
      requireValue(
        (result.requestId === undefined || result.requestId === id) &&
          ['pending', 'replied', 'unavailable'].includes(result.state),
      );
      if (result.state === 'replied') requireValue(typeof result.message === 'string');
      return { ...structuredClone(result), requestId: id } as ResultState;
    },
    check: async () => {
      throw new Error('Remote check is unavailable.');
    },
  };
}
