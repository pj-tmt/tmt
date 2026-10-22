import { base64url, envelopeSigningBytes } from './canonical-bytes.js';
import type { Session } from './device.js';
import { channelFor, verifyResponse, type Channel } from './session-channel.js';

export interface SendInput {
  operationId: string;
  agentId: string;
  message: string;
}
/** How the serve attributed a send, present only when there is something to say (#4255): it went
 * out anonymously because Core refused the person's name (the message is Core's own and names the
 * fix), or Core resolved the person under a `-user` name because an agent holds the plain one. */
export type SendIdentity =
  | {
      state: 'anonymous';
      reason: 'USER_NAME_UNAVAILABLE' | 'USER_NAME_INVALID' | 'USER_NAME_TAKEN';
      message: string;
    }
  | { state: 'person'; name: string; renamedFrom: string };
export type SendState = (
  | { state: 'held'; operationId: string }
  | { state: 'accepted'; operationId: string; requestId: string }
  | { state: 'uncertain'; operationId: string; requestId?: string }
  | {
      state: 'refused' | 'cancelled';
      operationId: string;
      reason?: string;
      limit?: number;
      settingsUrl?: string;
      scope?: 'talk';
    }
) & { identity?: SendIdentity };
/** Core's message state for a request, forwarded exactly (#4378); a state this SDK does not know is
 * left out of the reply rather than failing it. `canSendNow` is display only:
 * the owner's browser sees Core's answer, every other device sees `false`, and the server refuses a
 * send-now from it regardless. `reason.code` is an open set; show `reason.text` for an unknown one. */
export interface MessageState {
  state: 'sent' | 'batched' | 'delivered' | 'working' | 'unconfirmed' | 'not_sent';
  reason: { code: string; text: string };
  latestMs: number | null;
  remainingMs: number | null;
  canSendNow: boolean;
}
export type ResultState =
  | { state: 'pending'; requestId: string; messageState?: MessageState }
  | { state: 'replied'; requestId: string; message: string }
  | { state: 'unavailable'; requestId: string; reason?: string };
/** What asking to send a message now came to. `sent` only when Core typed it; `unconfirmed` means
 * it may have been typed, so do not offer it again before reading `result`; `unavailable` and
 * `refused` typed nothing. An unrecognized state decodes as `unconfirmed`, never `sent`. */
export type SendNowState = {
  state: 'sent' | 'already' | 'refused' | 'unconfirmed' | 'unavailable';
  requestId: string;
  reason?: string;
  messageState?: MessageState;
};
export interface RemoteAgent {
  id: string;
  name: string;
  presence: 'active' | 'offline' | 'unknown';
  /** Core owns this additive projection; forward its published value unchanged. */
  delivery?: unknown;
  /** Current verified runtime evidence; recognizing a new driver needs an additive SDK release. */
  runningDriver?: 'claude' | 'codex';
  /** Advisory Core activity, not readiness or completion evidence. */
  activity?: { state: 'working' | 'idle' | 'unknown' | 'ended'; sinceMs: number | null };
  /** Owner wall clock at the Core read; compare sinceMs with this, not the viewer clock. */
  observedAtMs?: number;
}
export interface RemoteOperations {
  listAgents(): Promise<RemoteAgent[]>;
  send(input: SendInput): Promise<SendState>;
  operation(operationId: string): Promise<SendState>;
  result(requestId: string): Promise<ResultState>;
  /** The owner's own browser only, for a request this device sent; others get a RefusalError
   * (`REMOTE_SCOPE_DENIED`). A repeat is safe: a delivered message answers `already`. A thrown
   * ClientError means the outcome is unknown: read `result` before offering it again. */
  sendNow(requestId: string): Promise<SendNowState>;
}
export type ClientErrorCode =
  | 'transport_failure'
  | 'timeout'
  | 'unverifiable_response'
  | 'sequence_unavailable'
  | 'outcome_unconfirmed';
export type RemoteRefusalCode =
  | 'REMOTE_SCOPE_DENIED'
  | 'REMOTE_INPUT_INVALID'
  | 'REMOTE_RATE_LIMITED'
  | 'REMOTE_INTENT_CONFLICT'
  | 'REMOTE_GRANT_CHANGED'
  | 'REMOTE_SERVE_STOPPED'
  | 'REMOTE_CLOSED'
  | 'REMOTE_SESSION_ENDED'
  | 'REMOTE_SESSION_EVICTED'
  | 'REMOTE_SESSION_LIMIT'
  | 'REMOTE_INPUT_TOO_LARGE'
  | 'REMOTE_STATE_UNAVAILABLE'
  | 'REMOTE_CORE_UNAVAILABLE'
  | 'REMOTE_MANAGEMENT_READ_ONLY'
  | 'REMOTE_MANAGEMENT_UNAVAILABLE'
  | 'REMOTE_GRANT_EXPIRED'
  | 'REMOTE_DEVICE_REVOKED'
  | 'REMOTE_DEVICE_NOT_FOUND'
  | 'REMOTE_SETTINGS_UNAVAILABLE'
  | 'REMOTE_MANAGEMENT_CAPACITY'
  | 'REMOTE_DEVICE_LIST_CHANGED'
  | 'REMOTE_DEVICE_REMOVAL_LIMIT';
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
    readonly limit?: number,
    readonly settingsUrl?: string,
    readonly scope?: 'talk',
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
// Only these signed errors prove management did not reach an effect. Generic
// state/ownership/session failures may come from response publication after commit.
const MANAGEMENT_PRE_EFFECT = new Set<string>([
  'REMOTE_SCOPE_DENIED',
  'REMOTE_INPUT_INVALID',
  'REMOTE_RATE_LIMITED',
  'REMOTE_INTENT_CONFLICT',
  'REMOTE_INPUT_TOO_LARGE',
  'REMOTE_MANAGEMENT_READ_ONLY',
  'REMOTE_DEVICE_REVOKED',
  'REMOTE_DEVICE_NOT_FOUND',
  'REMOTE_SETTINGS_UNAVAILABLE',
  'REMOTE_MANAGEMENT_CAPACITY',
  'REMOTE_DEVICE_LIST_CHANGED',
  'REMOTE_DEVICE_REMOVAL_LIMIT',
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
const MESSAGE_STATES = ['sent', 'batched', 'delivered', 'working', 'unconfirmed', 'not_sent'];
const timestamp = (value: unknown): value is number | null =>
  value === null || (typeof value === 'number' && Number.isSafeInteger(value) && value >= 0);
/** Optional, and exact when present: a state that does not parse makes the whole reply unverifiable.
 * A well-formed state this SDK does not know is dropped and the rest of the reply kept, so a new
 * Core state needs an additive SDK release but never breaks result(). */
function messageState(value: Record<string, unknown>): { messageState?: MessageState } {
  if (value.messageState === undefined) return {};
  const state = record(value.messageState);
  if (typeof state.state === 'string' && !MESSAGE_STATES.includes(state.state)) return {};
  valid(
    Object.keys(state).sort().join() === 'canSendNow,latestMs,reason,remainingMs,state' &&
      MESSAGE_STATES.includes(state.state as string) &&
      typeof state.canSendNow === 'boolean' &&
      timestamp(state.latestMs) &&
      timestamp(state.remainingMs),
  );
  const why = record(state.reason);
  valid(
    Object.keys(why).sort().join() === 'code,text' &&
      typeof why.code === 'string' &&
      /^[a-z][a-z0-9_]{0,63}$/.test(why.code) &&
      typeof why.text === 'string' &&
      utf8.encode(why.text).length <= 256,
  );
  return {
    messageState: {
      state: state.state as MessageState['state'],
      reason: { code: why.code, text: why.text },
      latestMs: state.latestMs as number | null,
      remainingMs: state.remainingMs as number | null,
      canSendNow: state.canSendNow,
    },
  };
}
const USER_NAME_REASONS = ['USER_NAME_UNAVAILABLE', 'USER_NAME_INVALID', 'USER_NAME_TAKEN'];
function text(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && utf8.encode(value).length <= 512;
}
/** Optional, and exact when present: a notice that does not parse makes the whole reply unverifiable. */
function identity(value: Record<string, unknown>): { identity?: SendIdentity } {
  if (value.identity === undefined) return {};
  const notice = record(value.identity);
  const keys = Object.keys(notice).sort().join();
  if (notice.state === 'anonymous') {
    valid(keys === 'message,reason,state' && USER_NAME_REASONS.includes(notice.reason as string));
    valid(text(notice.message));
    return {
      identity: {
        state: 'anonymous',
        reason: notice.reason as 'USER_NAME_UNAVAILABLE' | 'USER_NAME_INVALID' | 'USER_NAME_TAKEN',
        message: notice.message,
      },
    };
  }
  valid(notice.state === 'person' && keys === 'name,renamedFrom,state');
  valid(text(notice.name) && text(notice.renamedFrom));
  return { identity: { state: 'person', name: notice.name, renamedFrom: notice.renamedFrom } };
}
function sendState(value: unknown, id: string): SendState {
  return { ...sendStateBase(value, id), ...identity(record(value)) };
}
function sendStateBase(value: unknown, id: string): SendState {
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
      return { state: 'pending', requestId: id, ...messageState(state) };
    case 'replied':
      valid(typeof state.message === 'string');
      return { state: 'replied', requestId: id, message: state.message };
    case 'unavailable':
      return { state: 'unavailable', requestId: id, ...reason(state) };
    default:
      throw new Error('Invalid result state.');
  }
}
const SEND_NOW_STATES = ['sent', 'already', 'refused', 'unconfirmed', 'unavailable'];
/** The reply to a send-now ask. The request ID must be the asked one; an unknown state is
 * `unconfirmed`, so a future state can never read as `sent`. */
function sendNowState(value: unknown, id: string): SendNowState {
  const state = record(value);
  valid(state.requestId === id);
  const known = SEND_NOW_STATES.includes(state.state as string);
  return {
    state: known ? (state.state as SendNowState['state']) : 'unconfirmed',
    requestId: id,
    ...reason(state),
    ...messageState(state),
  };
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
    const activity = row.activity;
    const observedAtMs = row.observedAtMs;
    return {
      id: row.id as string,
      name: row.name as string,
      presence: row.presence as RemoteAgent['presence'],
      ...(Object.hasOwn(row, 'delivery') ? { delivery: row.delivery } : {}),
      ...(row.runningDriver === 'claude' || row.runningDriver === 'codex'
        ? { runningDriver: row.runningDriver }
        : {}),
      ...(activity !== null &&
      typeof activity === 'object' &&
      !Array.isArray(activity) &&
      'state' in activity &&
      'sinceMs' in activity &&
      (activity.state === 'working' ||
        activity.state === 'idle' ||
        activity.state === 'unknown' ||
        activity.state === 'ended') &&
      (activity.sinceMs === null ||
        (typeof activity.sinceMs === 'number' &&
          Number.isSafeInteger(activity.sinceMs) &&
          activity.sinceMs >= 0))
        ? { activity: { state: activity.state, sinceMs: activity.sinceMs } }
        : {}),
      ...(typeof observedAtMs === 'number' &&
      Number.isSafeInteger(observedAtMs) &&
      observedAtMs >= 0
        ? { observedAtMs }
        : {}),
    };
  });
}
export function remoteError(
  value: Record<string, unknown>,
  address: string,
): RefusalError | SequenceMismatch | undefined {
  if (!Object.hasOwn(value, 'error')) return undefined;
  const error = record(value.error);
  valid(
    typeof error.code === 'string' &&
      /^[A-Z][A-Z0-9_]{0,63}$/.test(error.code) &&
      typeof error.message === 'string',
  );
  const scope = error.scope;
  valid(scope === undefined || (error.code === 'REMOTE_SCOPE_DENIED' && scope === 'talk'));
  const limit = error.limit;
  valid(
    limit === undefined || (typeof limit === 'number' && Number.isSafeInteger(limit) && limit > 0),
  );
  valid(
    !['REMOTE_SESSION_EVICTED', 'REMOTE_SESSION_LIMIT'].includes(error.code) || limit !== undefined,
  );
  let settingsUrl: string | undefined;
  if (error.settingsUrl !== undefined && error.settingsUrl !== null) {
    valid(
      typeof error.settingsUrl === 'string' &&
        error.settingsUrl.length > 0 &&
        error.settingsUrl.length <= 2048,
    );
    const url = new URL(error.settingsUrl as string, address);
    const door = new URL(address);
    valid(
      url.origin === door.origin &&
        !url.username &&
        !url.password &&
        ['http:', 'https:'].includes(url.protocol),
    );
    settingsUrl = url.href;
  }
  const retry = error.retryAfterMs;
  valid(
    retry === undefined ||
      (typeof retry === 'number' && Number.isSafeInteger(retry) && retry >= 0 && retry <= 60000),
  );
  if (error.code === 'REMOTE_REPLAY') return new SequenceMismatch();
  valid(
    PRE_EFFECT.has(error.code) ||
      [
        'REMOTE_GRANT_CHANGED',
        'REMOTE_SERVE_STOPPED',
        'REMOTE_SESSION_LIMIT',
        'REMOTE_INPUT_TOO_LARGE',
        'REMOTE_STATE_UNAVAILABLE',
        'REMOTE_CORE_UNAVAILABLE',
        'REMOTE_MANAGEMENT_READ_ONLY',
        'REMOTE_MANAGEMENT_UNAVAILABLE',
        'REMOTE_GRANT_EXPIRED',
        'REMOTE_DEVICE_REVOKED',
        'REMOTE_DEVICE_NOT_FOUND',
        'REMOTE_SETTINGS_UNAVAILABLE',
        'REMOTE_MANAGEMENT_CAPACITY',
        'REMOTE_DEVICE_LIST_CHANGED',
        'REMOTE_DEVICE_REMOVAL_LIMIT',
      ].includes(error.code),
  );
  return new RefusalError(
    error.code as RemoteRefusalCode,
    retry as number | undefined,
    limit as number | undefined,
    settingsUrl,
    scope as 'talk' | undefined,
  );
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
  mutationOutcome = false,
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
          if (mutationOutcome) throw new Error('Mutation acknowledgment is unconfirmed.');
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
        const error = remoteError(value, channel.paired.address);
        if (error) {
          if (
            error instanceof RefusalError &&
            [
              'REMOTE_CLOSED',
              'REMOTE_GRANT_CHANGED',
              'REMOTE_SERVE_STOPPED',
              'REMOTE_SESSION_ENDED',
              'REMOTE_SESSION_EVICTED',
            ].includes(error.code)
          )
            channel.ended = true;
          if (
            mutationOutcome &&
            error instanceof RefusalError &&
            !MANAGEMENT_PRE_EFFECT.has(error.code)
          ) {
            failure = 'outcome_unconfirmed';
            throw new Error('Signed failure does not establish the management outcome.');
          }
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
      outcome_unconfirmed:
        'Remote management outcome is unconfirmed; observe the original operation.',
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

export function parseCapabilities(value: unknown): void {
  const reply = record(value);
  valid(reply.version === 1 && reply.profile === 'local-v1' && reply.binding === 'loopback-http');
  valid(
    Array.isArray(reply.operations) && reply.operations.every((value) => typeof value === 'string'),
  );
  record(reply.limits);
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

  try {
    try {
      await attempt(
        channel,
        timeoutMs,
        'capabilities',
        id,
        payload,
        channel.clientSequence,
        parseCapabilities,
      );
    } catch (error) {
      if (!(error instanceof SequenceMismatch)) throw error;
      await attempt(channel, timeoutMs, 'capabilities', id, payload, unresolved, parseCapabilities);
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
  mutationOutcome = false,
): Promise<T> {
  if (channel.ended || channel.clientSequence > MAX_SEQUENCE)
    throw new RefusalError('REMOTE_SESSION_ENDED');
  await synchronize(channel, timeoutMs);
  if (channel.clientSequence > MAX_SEQUENCE) throw new RefusalError('REMOTE_SESSION_ENDED');
  const sequence = channel.clientSequence;
  try {
    return await attempt(
      channel,
      timeoutMs,
      operation,
      id,
      payload,
      sequence,
      parse,
      mutationOutcome,
    );
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

/** Package-internal primitive shared by agent and Remote management helpers.
 * It retains the verified Session's serialized lane, signer and response correlation.
 */
export function verifiedSessionRequest(session: Session, options: { timeoutMs?: number } = {}) {
  const found = channelFor(session);
  if (!found) throw new TypeError('Use a verified openSession or reopenSession result.');
  const channel = found;
  const timeoutMs = options.timeoutMs ?? 40000;
  input(Number.isSafeInteger(timeoutMs) && timeoutMs > 0 && timeoutMs <= 2147483647);
  return function call<T>(
    operation: string,
    id: string,
    value: unknown,
    parse: (value: unknown) => T,
    mutationOutcome = false,
  ): Promise<T> {
    const payload = utf8.encode(JSON.stringify(value));
    return enqueue(channel, () =>
      invoke(channel, timeoutMs, operation, id, payload, parse, mutationOutcome),
    );
  };
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
  const call = verifiedSessionRequest(session, options);
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
          return {
            state: 'refused',
            operationId,
            reason: error.code,
            ...(error.limit === undefined ? {} : { limit: error.limit }),
            ...(error.settingsUrl === undefined ? {} : { settingsUrl: error.settingsUrl }),
            ...(error.scope === undefined ? {} : { scope: error.scope }),
          };
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
          return {
            state: 'refused',
            operationId,
            reason: error.code,
            ...(error.limit === undefined ? {} : { limit: error.limit }),
            ...(error.settingsUrl === undefined ? {} : { settingsUrl: error.settingsUrl }),
            ...(error.scope === undefined ? {} : { scope: error.scope }),
          };
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
    async sendNow(id) {
      input(requestId(id));
      return call(
        'requests.sendNow',
        crypto.randomUUID(),
        { version: 1, operation: 'requests.sendNow', input: { requestId: id } },
        (value) => sendNowState(value, id),
      );
    },
  };
}
