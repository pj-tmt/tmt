import crypto from 'node:crypto';
import Database from 'better-sqlite3';

export const MAX_RESPONSE_BYTES = 1_048_576;
export interface ResponseEndpoint {
  serverId: string;
  socketPath: string;
  serverPid: number;
  serverStartTime: string;
  paneId: string;
  panePid: number;
}
export const RESPONSE_SERVER = {
  serverId: 'native-response-server',
  socketPath: '/tmp/native-response.sock',
  serverPid: 1234,
  serverStartTime: 'native-response-start',
  paneId: '%1',
  panePid: 5678,
} as const;

const DAY_MS = 86_400_000;

export interface SeededResponse {
  readonly requestId: string;
  readonly attemptId: string;
  readonly preparedAtMs: number;
  readonly v1Receipt: string;
  readonly compactReceipt: string;
}

export interface SeedResponseOptions {
  readonly nowMs?: number;
  readonly status?: 'prepared' | 'sent';
  readonly expired?: boolean;
}

function appendString(parts: Buffer[], value: string): void {
  const bytes = Buffer.from(value, 'utf8');
  const length = Buffer.alloc(8);
  length.writeBigUInt64BE(BigInt(bytes.byteLength));
  parts.push(length, bytes);
}

/** Independently derive the native compact receipt protocol. */
export function compactReceipt(
  requestId: string,
  attemptId: string,
  endpoint: ResponseEndpoint = RESPONSE_SERVER
): string {
  const parts = [Buffer.from('tmt/reply-receipt/v2\0', 'utf8')];
  appendString(parts, requestId);
  appendString(parts, attemptId);
  appendString(parts, endpoint.serverId);
  appendString(parts, endpoint.socketPath);
  const serverPid = Buffer.alloc(8);
  serverPid.writeBigUInt64BE(BigInt(endpoint.serverPid));
  parts.push(serverPid);
  appendString(parts, endpoint.serverStartTime);
  appendString(parts, endpoint.paneId);
  const panePid = Buffer.alloc(8);
  panePid.writeBigUInt64BE(BigInt(endpoint.panePid));
  parts.push(panePid);
  return `v2_${crypto.createHash('sha256').update(Buffer.concat(parts)).digest().subarray(0, 16).toString('base64url')}`;
}

/** Produce the installed TypeScript receipt envelope without importing its codec. */
export function v1Receipt(
  requestId: string,
  attemptId: string,
  endpoint: ResponseEndpoint = RESPONSE_SERVER
): string {
  return Buffer.from(
    JSON.stringify({
      version: 1,
      requestId,
      attemptId,
      endpoint,
    }),
    'utf8'
  ).toString('base64url');
}

function withDatabase<T>(
  file: string,
  callback: (database: Database.Database) => T,
  readonly = false
): T {
  const database = new Database(file, { readonly });
  try {
    database.pragma('foreign_keys = ON');
    return callback(database);
  } finally {
    database.close();
  }
}

export function seedResponse(
  file: string,
  requestId: string,
  options: SeedResponseOptions = {}
): SeededResponse {
  const nowMs = options.nowMs ?? Date.now();
  // The native service only rejects an expired attempt after the frozen
  // seven-day acceptance window has elapsed. Keep the row retained while
  // moving its prepared/deadline timestamps far enough into the past.
  const preparedAtMs = options.expired ? nowMs - 8 * DAY_MS : nowMs;
  const attemptId = `attempt-${requestId}`;
  const expiresAtMs = options.expired ? preparedAtMs - 1 : preparedAtMs + 60 * 60 * 1000;
  const retentionExpiresAtMs = nowMs + 7 * DAY_MS;
  const status = options.status ?? 'sent';
  withDatabase(file, (database) => {
    database
      .prepare(
        `INSERT INTO request_attempts (
           attempt_id, request_id, nonce, identity_id, server_id, socket_path,
           server_pid, server_start_time, pane_id, pane_pid, wait_active, status,
           preamble_every, inject_preamble, cadence_reserved, prepared_at_ms,
           sending_at_ms, settled_at_ms, wait_released_at_ms, response_submitted_at_ms,
           expires_at_ms, retention_days, retention_expires_at_ms, originator_kind,
           originator_identity_id, recipient_identity_id, message_text, message_bytes,
           message_expires_at_ms, attention_revision, attention_acknowledged_revision
         ) VALUES (?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, NULL, 0, 0, ?, ?, ?, NULL, NULL,
                   ?, 7, ?, 'unknown', NULL, NULL, ?, ?, ?, 0, 0)`
      )
      .run(
        attemptId,
        requestId,
        `nonce-${requestId}`,
        RESPONSE_SERVER.serverId,
        RESPONSE_SERVER.socketPath,
        RESPONSE_SERVER.serverPid,
        RESPONSE_SERVER.serverStartTime,
        RESPONSE_SERVER.paneId,
        RESPONSE_SERVER.panePid,
        status === 'sent' ? 0 : 1,
        status,
        preparedAtMs,
        status === 'prepared' ? null : preparedAtMs + 1,
        status === 'prepared' ? null : preparedAtMs + 2,
        expiresAtMs,
        retentionExpiresAtMs,
        `prompt for ${requestId}`,
        Buffer.byteLength(`prompt for ${requestId}`),
        retentionExpiresAtMs
      );
  });
  return {
    requestId,
    attemptId,
    preparedAtMs,
    v1Receipt: v1Receipt(requestId, attemptId),
    compactReceipt: compactReceipt(requestId, attemptId),
  };
}

export function schemaVersion(file: string): number {
  return withDatabase(
    file,
    (database) => {
      const row = database
        .prepare('SELECT COALESCE(MAX(version), 0) AS version FROM _migrations')
        .get() as { version: number };
      return row.version;
    },
    true
  );
}

export function removeAttempt(file: string, requestId: string): void {
  withDatabase(file, (database) => {
    const result = database
      .prepare('DELETE FROM request_attempts WHERE request_id = ?')
      .run(requestId);
    if (result.changes !== 1) throw new Error('Expected exactly one retained attempt to remove.');
  });
}

export function responseSnapshot(
  file: string,
  requestId: string
): {
  readonly attempt: Record<string, unknown> | undefined;
  readonly response: Record<string, unknown> | undefined;
} {
  return withDatabase(
    file,
    (database) => ({
      attempt: database
        .prepare(
          `SELECT request_id, attempt_id, status, wait_active, preamble_every,
                inject_preamble, cadence_reserved, prepared_at_ms, sending_at_ms,
                settled_at_ms, wait_released_at_ms, response_submitted_at_ms,
                expires_at_ms, retention_days, retention_expires_at_ms,
                attention_revision, attention_acknowledged_revision,
                message_text, message_bytes, message_expires_at_ms
           FROM request_attempts WHERE request_id = ?`
        )
        .get(requestId) as Record<string, unknown> | undefined,
      response: database
        .prepare(
          `SELECT request_id, attempt_id, body, body_bytes, submitted_at_ms,
                response_expires_at_ms
           FROM request_responses WHERE request_id = ?`
        )
        .get(requestId) as Record<string, unknown> | undefined,
    }),
    true
  );
}
