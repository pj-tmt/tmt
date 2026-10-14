import Database from 'better-sqlite3';
import path from 'node:path';
import type { E2EFixture } from './harness.js';

export interface DurableIdentityRow extends Record<string, unknown> {
  id: string;
  name: string;
  lifetime: 'temporary' | 'saved';
  retired_at_ms: number | null;
}

export interface DurableState {
  identities: DurableIdentityRow[];
  bindings: Array<Record<string, unknown>>;
  profiles: Array<Record<string, unknown>>;
  metadata: Array<Record<string, unknown>>;
}

/** Observe committed identity state without service reconciliation or writes. */
export function durableState(fixture: E2EFixture): DurableState {
  const database = new Database(path.join(fixture.globalDir, 'tmt.db'), { readonly: true });
  try {
    return {
      identities: database
        .prepare('SELECT * FROM identities ORDER BY canonical_name')
        .all() as DurableIdentityRow[],
      bindings: database.prepare('SELECT * FROM bindings ORDER BY identity_id').all() as Array<
        Record<string, unknown>
      >,
      profiles: database.prepare('SELECT * FROM role_profiles ORDER BY identity_id').all() as Array<
        Record<string, unknown>
      >,
      metadata: database
        .prepare('SELECT * FROM identity_metadata ORDER BY identity_id, key')
        .all() as Array<Record<string, unknown>>,
    };
  } finally {
    database.close();
  }
}

/** Correlate public identity projections with an independent, unretired SQL row. */
export function durableIdentity(fixture: E2EFixture, name: string): DurableIdentityRow {
  const row = durableState(fixture).identities.find(
    (identity) => identity.name === name && identity.retired_at_ms === null
  );
  if (!row) throw new Error(`Missing unretired fixture identity: ${name}`);
  return row;
}

/** Ignore only the verification clock; retain every other durable binding field. */
export function withoutVerificationTimestamp(
  bindings: Array<Record<string, unknown>>
): Array<Record<string, unknown>> {
  return bindings
    .map(({ last_verified_at: _lastVerifiedAt, ...binding }) => binding)
    .sort((left, right) => String(left.id).localeCompare(String(right.id)));
}
