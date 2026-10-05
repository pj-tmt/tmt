import { base64url, envelopeSigningBytes } from './canonical-bytes.js';
import type { Session } from './device.js';
import { channelFor, verifyResponse, type Channel } from './session-channel.js';

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
export interface RemoteAgent {
  id: string;
  name: string;
  presence: 'active' | 'offline' | 'unknown';
  /** Core owns this additive projection; forward its published value unchanged. */
  delivery?: unknown;
}
export interface RemoteOperations {
  listAgents(): Promise<RemoteAgent[]>;
  send(input: SendInput): Promise<SendState>;
  operation(operationId: string): Promise<SendState>;
  result(requestId: string): Promise<ResultState>;
}
export type ClientErrorCode =
  | 'transport_failure'
  | 'timeout'
  | 'unverifiable_response'
  | 'sequence_unavailable';
export type RemoteRefusalCode =
  | 'REMOTE_SCOPE_DENIED'
  | 'REMOTE_INPUT_INVALID'
  | 'REMOTE_RATE_LIMITED'
  | 'REMOTE_INTENT_CONFLICT'
  | 'REMOTE_CLOSED'
  | 'REMOTE_SESSION_ENDED'
  | 'REMOTE_SESSION_EVICTED'
  | 'REMOTE_INPUT_TOO_LARGE'
  | 'REMOTE_STATE_UNAVAILABLE'
  | 'REMOTE_CORE_UNAVAILABLE';
/** Unknown outcome only; the caller retains its operation ID for read-only recovery. */
export class ClientError extends Error {
  constructor(
    readonly code: ClientErrorCode,
    message: string,
    readonly operationId?: string,
  ) {
    super(message);
    this.name = 'ClientError';
  }
}
/** Known refusal; send/operation return pre-effect codes as state instead. */
export class RefusalError extends Error {
  constructor(
    readonly code: RemoteRefusalCode,
    readonly retryAfterMs?: number,
  ) {
    super(`Remote operation refused: ${code}.`);
    this.name = 'RefusalError';
  }
}
class SequenceMismatch extends Error {}
const PRE_EFFECT = new Set<string>([
  'REMOTE_SCOPE_DENIED',
  'REMOTE_INPUT_INVALID',
  'REMOTE_RATE_LIMITED',
  'REMOTE_INTENT_CONFLICT',
  'REMOTE_CLOSED',
  'REMOTE_SESSION_ENDED',
  'REMOTE_SESSION_EVICTED',
]);
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const coreId = (value: unknown): value is string =>
  typeof value === 'string' && UUID.test(value) && value !== '00000000-0000-0000-0000-000000000000';
const requestId = (value: unknown): value is string =>
  typeof value === 'string' && value.startsWith('req_') && coreId(value.slice(4));
const utf8 = new TextEncoder();
const strictUtf8 = new TextDecoder('utf-8', { fatal: true });
const MAX_SEQUENCE = 18446744073709551615n;
function valid(condition: boolean): asserts condition {
  if (!condition) throw new Error('Invalid operation response.');
}
function input(condition: boolean): asserts condition {
  if (!condition) throw new TypeError('Invalid Remote operation input.');
}
function record(value: unknown): Record<string, unknown> {
  valid(typeof value === 'object' && value !== null && !Array.isArray(value));
  return value as Record<string, unknown>;
}
function reason(value: Record<string, unknown>): { reason?: string } {
  if (value.reason === undefined) return {};
  valid(typeof value.reason === 'string' && utf8.encode(value.reason).length <= 256);
  return { reason: value.reason };
}
function sendState(value: unknown, id: string): SendState {
  const state = record(value);
  valid(state.operationId === id);
  switch (state.state) {
    case 'held':
      return { state: 'held', operationId: id };
    case 'accepted':
      valid(requestId(state.requestId));
      return { state: 'accepted', operationId: id, requestId: state.requestId };
    case 'uncertain':
      valid(state.requestId === undefined || requestId(state.requestId));
      return {
        state: 'uncertain',
        operationId: id,
        ...(state.requestId === undefined ? {} : { requestId: state.requestId }),
      };
    case 'refused':
    case 'cancelled':
      return { state: state.state, operationId: id, ...reason(state) };
    default:
      throw new Error('Invalid send state.');
  }
}
function resultState(value: unknown, id: string): ResultState {
  const state = record(value);
  valid(state.requestId === id);
  switch (state.state) {
    case 'pending':
      return { state: 'pending', requestId: id };
    case 'replied':
      valid(typeof state.message === 'string');
      return { state: 'replied', requestId: id, message: state.message };
    case 'unavailable':
      return { state: 'unavailable', requestId: id, ...reason(state) };
    default:
      throw new Error('Invalid result state.');
  }
}
function agents(value: unknown): RemoteAgent[] {
  const rows = record(value).identities;
  valid(Array.isArray(rows));
  return rows.map((value: unknown) => {
    const row = record(value);
    valid(
      coreId(row.id) &&
        typeof row.name === 'string' &&
        (row.presence === 'active' || row.presence === 'offline' || row.presence === 'unknown'),
    );
    return {
      id: row.id as string,
      name: row.name as string,
      presence: row.presence as RemoteAgent['presence'],
      ...(Object.hasOwn(row, 'delivery') ? { delivery: row.delivery } : {}),
    };
  });
}
function remoteError(value: Record<string, unknown>): RefusalError | SequenceMismatch | undefined {
  if (!Object.hasOwn(value, 'error')) return undefined;
  const error = record(value.error);
  valid(
    typeof error.code === 'string' &&
      /^[A-Z][A-Z0-9_]{0,63}$/.test(error.code) &&
      typeof error.message === 'string',
  );
  const retry = error.retryAfterMs;
  valid(
    retry === undefined ||
      (typeof retry === 'number' && Number.isSafeInteger(retry) && retry >= 0 && retry <= 60000),
  );
  if (error.code === 'REMOTE_REPLAY') return new SequenceMismatch();
  valid(
    PRE_EFFECT.has(error.code) ||
      ['REMOTE_INPUT_TOO_LARGE', 'REMOTE_STATE_UNAVAILABLE', 'REMOTE_CORE_UNAVAILABLE'].includes(
        error.code,
      ),
  );
  return new RefusalError(error.code as RemoteRefusalCode, retry as number | undefined);
}
function enqueue<T>(channel: Channel, action: () => Promise<T>): Promise<T> {
  const pending = channel.tail.then(action);
  channel.tail = pending.then(
    () => undefined,
    () => undefined,
  );
  return pending;
}
/** One attempt; an abandoned fetch can finish without touching live counters. */
async function attempt<T>(
  channel: Channel,
  timeoutMs: number,
  operation: string,
  id: string,
  payload: Uint8Array,
  sequence: bigint,
  parse: (value: unknown) => T,
): Promise<T> {
  const abort = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let published = false;
  let failure: ClientErrorCode = 'transport_failure';
  try {
    return await Promise.race([
      (async () => {
        const envelope = {
          version: 1 as const,
          profile: 'local-v1' as const,
          kind: 'request' as const,
          id,
          correlationId: null,
          machineId: channel.paired.machineId,
          windowId: channel.windowId,
          clientId: channel.paired.clientId,
          sessionId: channel.sessionId,
          sequence: sequence.toString(),
          timestampMs: Date.now(),
          origin: channel.paired.origin,
          operation,
        };
        const signature = await channel.key.sign(
          await envelopeSigningBytes({ ...envelope, payload }),
        );
        if (abort.signal.aborted) throw new Error('Abandoned signing.');
        published = true;
        channel.clientSequence = sequence + 1n;
        const send = channel.fetch;
        const response = await send(`${channel.paired.address}/append`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          credentials: 'omit',
          signal: abort.signal,
          body: JSON.stringify({
            ...envelope,
            payload: base64url(payload),
            signature: base64url(signature),
          }),
        });
        if (abort.signal.aborted) throw new Error('Abandoned response.');
        if (response.status === 404) {
          channel.ended = true;
          throw new RefusalError('REMOTE_SESSION_ENDED');
        }
        if (response.status !== 200) throw new Error('Unconfirmed transport response.');
        failure = 'unverifiable_response';
        const reply = await verifyResponse(await response.json(), channel.paired, {
          id,
          windowId: channel.windowId,
          operation,
          sessionId: channel.sessionId,
          after: channel.machineSequence,
        });
        if (abort.signal.aborted) throw new Error('Abandoned response.');
        channel.machineSequence = reply.sequence;
        const value = record(JSON.parse(strictUtf8.decode(reply.payload)) as unknown);
        const error = remoteError(value);
        if (error) {
          if (
            error instanceof RefusalError &&
            ['REMOTE_CLOSED', 'REMOTE_SESSION_ENDED', 'REMOTE_SESSION_EVICTED'].includes(error.code)
          )
            channel.ended = true;
          throw error;
        }
        return parse(value);
      })(),
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => {
          failure = 'timeout';
          abort.abort();
          reject(new Error('Timed out.'));
        }, timeoutMs);
      }),
    ]);
  } catch (error) {
    if (error instanceof RefusalError || error instanceof SequenceMismatch) throw error;
    if (!published) throw new TypeError('Remote operation could not be signed.');
    const messages: Record<ClientErrorCode, string> = {
      timeout: 'Remote outcome is unknown after timeout; observe the original operation.',
      transport_failure: 'Remote transport outcome is unknown; observe the original operation.',
      unverifiable_response:
        'Remote response could not be verified; observe the original operation.',
      sequence_unavailable:
        'Remote sequence recovery failed; reopen before observing the original operation.',
    };
    throw new ClientError(failure, messages[failure]);
  } finally {
    clearTimeout(timer);
    abort.abort();
  }
}

/** Scope-free reads synchronize the lane; the caller's operation is never a probe. */
async function synchronize(channel: Channel, timeoutMs: number): Promise<void> {
  const unresolved = channel.uncertainSequence;
  if (unresolved === undefined) return;
  const unavailable = () =>
    new ClientError(
      'sequence_unavailable',
      'Remote sequence recovery failed; reopen before observing the original operation.',
    );
  if (unresolved === 'unavailable') throw unavailable();
  const id = crypto.randomUUID();
  const payload = utf8.encode('{}');
  function capabilities(value: unknown): void {
    const reply = record(value);
    valid(reply.version === 1 && reply.profile === 'local-v1' && reply.binding === 'loopback-http');
    valid(
      Array.isArray(reply.operations) &&
        reply.operations.every((value) => typeof value === 'string'),
    );
    record(reply.limits);
  }
  try {
    try {
      await attempt(
        channel,
        timeoutMs,
        'capabilities',
        id,
        payload,
        channel.clientSequence,
        capabilities,
      );
    } catch (error) {
      if (!(error instanceof SequenceMismatch)) throw error;
      await attempt(channel, timeoutMs, 'capabilities', id, payload, unresolved, capabilities);
    }
    channel.uncertainSequence = undefined;
  } catch (error) {
    if (channel.ended) throw error;
    channel.uncertainSequence = 'unavailable';
    throw unavailable();
  }
}

async function invoke<T>(
  channel: Channel,
  timeoutMs: number,
  operation: string,
  id: string,
  payload: Uint8Array,
  parse: (value: unknown) => T,
): Promise<T> {
  if (channel.ended || channel.clientSequence > MAX_SEQUENCE)
    throw new RefusalError('REMOTE_SESSION_ENDED');
  await synchronize(channel, timeoutMs);
  if (channel.clientSequence > MAX_SEQUENCE) throw new RefusalError('REMOTE_SESSION_ENDED');
  const sequence = channel.clientSequence;
  try {
    return await attempt(channel, timeoutMs, operation, id, payload, sequence, parse);
  } catch (error) {
    if (error instanceof SequenceMismatch) {
      channel.uncertainSequence = 'unavailable';
      throw new ClientError(
        'sequence_unavailable',
        'Remote sequence recovery failed; reopen before observing the original operation.',
      );
    }
    // A signed refusal may precede sequence consumption (scope/rate checks),
    // or follow it (operation admission). A scope-free probe resolves either.
    if (!channel.ended && !(error instanceof TypeError)) channel.uncertainSequence = sequence;
    throw error;
  }
}

/**
 * Constructing this helper opens nothing. Reuse the owner's existing Session;
 * after unknown send outcomes, observe operation(originalId) in that session.
 * Recovery never dispatches, reopens, or automatically allocates a dispatch ID.
 */
export function operations(
  session: Session,
  options: { timeoutMs?: number } = {},
): RemoteOperations {
  const found = channelFor(session);
  if (!found) throw new TypeError('Use a verified openSession or reopenSession result.');
  const channel = found;
  const timeoutMs = options.timeoutMs ?? 40000;
  input(Number.isSafeInteger(timeoutMs) && timeoutMs > 0 && timeoutMs <= 2147483647);
  function call<T>(
    operation: string,
    id: string,
    value: unknown,
    parse: (value: unknown) => T,
  ): Promise<T> {
    const payload = utf8.encode(JSON.stringify(value));
    return enqueue(channel, () => invoke(channel, timeoutMs, operation, id, payload, parse));
  }
  return {
    listAgents: () => call('agents.list', crypto.randomUUID(), {}, agents),
    async send(value) {
      input(
        typeof value.operationId === 'string' &&
          V4.test(value.operationId) &&
          coreId(value.agentId) &&
          typeof value.message === 'string' &&
          strictUtf8.decode(utf8.encode(value.message)) === value.message,
      );
      const { operationId, agentId, message } = value;
      try {
        return await call(
          'dispatch.create',
          operationId,
          {
            version: 1,
            operation: 'dispatch.create',
            originator: 'anonymous',
            input: { operationId, recipientIds: [agentId], message, kind: 'request' },
          },
          (value) => sendState(value, operationId),
        );
      } catch (error) {
        if (error instanceof RefusalError && PRE_EFFECT.has(error.code))
          return { state: 'refused', operationId, reason: error.code };
        if (error instanceof ClientError)
          throw new ClientError(error.code, error.message, operationId);
        throw error;
      }
    },
    async operation(operationId) {
      input(coreId(operationId));
      try {
        return await call('operation.show', crypto.randomUUID(), { operationId }, (value) =>
          sendState(value, operationId),
        );
      } catch (error) {
        if (error instanceof RefusalError && PRE_EFFECT.has(error.code))
          return { state: 'refused', operationId, reason: error.code };
        if (error instanceof ClientError)
          throw new ClientError(error.code, error.message, operationId);
        throw error;
      }
    },
    async result(id) {
      input(requestId(id));
      return call('result', crypto.randomUUID(), { requestId: id }, (value) =>
        resultState(value, id),
      );
    },
  };
}
