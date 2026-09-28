import path from 'node:path';
import Database from 'better-sqlite3';
import type { E2EFixture } from './harness.js';

export interface AttemptRow {
  attempt_id: string;
  request_id: string;
  nonce: string | null;
  server_id: string;
  socket_path: string;
  server_pid: number;
  server_start_time: string;
  pane_id: string;
  pane_pid: number;
  wait_active: number;
  status: string;
  route_kind: 'pane' | 'inbox';
  wake_state: 'not_attempted' | 'claimed' | 'sent' | 'unavailable' | 'uncertain';
  inject_preamble: number;
  prepared_at_ms: number;
  retention_days: number;
  retention_expires_at_ms: number;
  originator_kind: 'unknown' | 'explicit' | 'verified';
  originator_identity_id: string | null;
  recipient_identity_id: string | null;
  room_id: string | null;
  message_text: string | null;
  message_bytes: number | null;
  message_expires_at_ms: number | null;
}

export interface ResponseRow {
  request_id: string;
  body: string;
  submitted_at_ms: number;
  response_expires_at_ms: number;
}

/** Read committed state independently of the CLI's service and connection. */
function read<T>(fixture: E2EFixture, query: (database: Database.Database) => T): T {
  const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
  try {
    return query(database);
  } finally {
    database.close();
  }
}

export function requestAttempts(fixture: E2EFixture): AttemptRow[] {
  return read(
    fixture,
    (database) => database.prepare('SELECT * FROM request_attempts').all() as AttemptRow[]
  );
}

export function preambleCounters(fixture: E2EFixture): Record<string, number> {
  return read(fixture, (database) => {
    const rows = database
      .prepare('SELECT identity_id, reserved_count FROM preamble_counters')
      .all() as Array<{ identity_id: string; reserved_count: number }>;
    return Object.fromEntries(rows.map((row) => [row.identity_id, row.reserved_count]));
  });
}

export function requestResponses(fixture: E2EFixture): ResponseRow[] {
  return read(
    fixture,
    (database) => database.prepare('SELECT * FROM request_responses').all() as ResponseRow[]
  );
}
