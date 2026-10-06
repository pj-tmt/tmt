import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vite-plus/test';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';

interface IdentityRow {
  id: string;
  name: string;
  canonical_name: string;
}

function identities(fixture: E2EFixture): IdentityRow[] {
  const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
  try {
    return database
      .prepare(
        'SELECT id, name, canonical_name FROM identities WHERE retired_at_ms IS NULL ORDER BY canonical_name'
      )
      .all() as IdentityRow[];
  } finally {
    database.close();
  }
}

function durableRetired(fixture: E2EFixture, canonicalName: string): boolean {
  const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
  try {
    const row = database
      .prepare('SELECT retired_at_ms FROM identities WHERE canonical_name = ?')
      .get(canonicalName) as { retired_at_ms: number | null } | undefined;
    return typeof row?.retired_at_ms === 'number';
  } finally {
    database.close();
  }
}

function bindingOwners(fixture: E2EFixture): string[] {
  const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
  try {
    const rows = database
      .prepare('SELECT identity_id FROM bindings ORDER BY identity_id')
      .all() as Array<{ identity_id: string }>;
    return rows.map((row) => row.identity_id);
  } finally {
    database.close();
  }
}

describe('committed identity retention', { concurrent: false }, () => {
  it('keeps an existing offline identity through a refused binding and reuses its UUID on a later verified binding', async () => {
    await withE2EFixture(async (fixture) => {
      expectJsonResult(await fixture.runJsonCli(['name', 'Occupied']));
      const initial = identities(fixture);
      const originalMetadata = fixture.paneMetadata();
      const originalTitle = fixture.paneTitle();

      // Both preflight failures must leave durable identity creation untouched.
      const invalid = await fixture.runJsonCli(['name', '%123']);
      expect(invalid.code).toBe(1);
      expect(invalid.json).toMatchObject({ error: { code: 'INVALID_NAME' } });
      const missing = await fixture.runJsonCli(['add', '%99999', 'MissingPane']);
      expect(missing.code).toBe(3);
      expect(missing.json).toMatchObject({ error: { code: 'PANE_NOT_FOUND' } });
      expect(identities(fixture)).toEqual(initial);

      // A refused bind of a brand-new name must not leave a never-bound row behind.
      const fresh = await fixture.runJsonCli(['name', 'Fresh']);
      expect(fresh.code).toBe(5);
      expect(fresh.json).toMatchObject({ error: { code: 'PANE_ALREADY_BOUND' } });
      expect(identities(fixture)).toEqual(initial);
      const retiredFresh = durableRetired(fixture, 'fresh');
      expect(retiredFresh).toBe(true);
      // `run` binds through the same path and is refused without launching anything.
      const refusedRun = await fixture.runCli(['run', 'FreshRun', '/bin/sh', '-c', 'exit 0']);
      expect(refusedRun.code).toBe(5);
      expect(identities(fixture)).toEqual(initial);
      expect(durableRetired(fixture, 'freshrun')).toBe(true);

      // An identity that already exists survives the same refusal, keeping its UUID.
      expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'Retained']));
      const conflict = await fixture.runJsonCli(['name', 'Retained']);
      expect(conflict.code).toBe(5);
      expect(conflict.stderr).toBe('');
      expect(conflict.json).toEqual({
        error: { code: 'PANE_ALREADY_BOUND', message: 'Pane is already bound to another name.' },
      });
      const rows = identities(fixture);
      expect(rows.map((row) => row.name)).toEqual(['Occupied', 'Retained']);
      const retained = rows[1];
      expect(retained.id).toMatch(/^[0-9a-f-]{36}$/);
      expect(rows[0]).toEqual(initial[0]);
      // Inspect before reconciliation could remove a wrongly published row.
      expect(bindingOwners(fixture)).toEqual([initial[0].id]);
      expect(fixture.paneMetadata()).toBe(originalMetadata);
      expect(fixture.paneTitle()).toBe(originalTitle);

      const list = expectJsonResult(
        await fixture.runJsonCli<{ identities: Array<{ name: string; presence: string }> }>([
          'list',
        ])
      );
      expect(list.identities.map(({ name, presence }) => ({ name, presence }))).toEqual([
        { name: 'Occupied', presence: 'active' },
        { name: 'Retained', presence: 'offline' },
      ]);
      const beforeEvents = fixture.events();
      const inactive = await fixture.runJsonCli([
        'talk',
        'Retained',
        'must not be delivered',
        '--detach',
      ]);
      expect(inactive.code).toBe(0);
      expect(inactive.json).toMatchObject({ status: 'queued', offline: true });
      expect(fixture.events()).toEqual(beforeEvents);

      const roleText = 'Data-only role; never inject this text.';
      const preamble = 'Retained identity context';
      const role = expectJsonResult(
        await fixture.runJsonCli<{
          identity: { id: string };
          role: { content: string };
        }>(['role', 'set', roleText, '--identity', 'Retained'], { withoutTmux: true })
      );
      expect(role.identity.id).toBe(retained.id);
      expect(role.role.content).toBe(roleText);
      expect(
        expectJsonResult(
          await fixture.runJsonCli(['preamble', 'set', 'Retained', preamble], {
            withoutTmux: true,
          })
        )
      ).toEqual({ agent: 'Retained', preamble, status: 'set' });

      const repeated = await fixture.runJsonCli(['name', 'RETAINED']);
      expect(repeated.code).toBe(5);
      expect(identities(fixture)).toEqual(rows);
      expect(bindingOwners(fixture)).toEqual([initial[0].id]);
      expect(fixture.paneMetadata()).toBe(originalMetadata);
      expect(fixture.paneTitle()).toBe(originalTitle);
      const peer = await fixture.createMockPane('retained-peer');
      expectJsonResult(await fixture.runJsonCli(['add', peer.pane, 'retained']));
      expect(identities(fixture)).toEqual(rows);
      expect(bindingOwners(fixture)).toEqual(rows.map((row) => row.id).sort());
      const reboundRole = expectJsonResult(
        await fixture.runJsonCli<{
          identity: { id: string };
          role: { content: string };
        }>(['role', 'show', '--identity', 'Retained'], { withoutTmux: true })
      );
      expect(reboundRole).toEqual(role);
      expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);

      const message = 'retained identity can now receive';
      const output = expectJsonResult(
        await fixture.runJsonCli<{ status: string; requestId: string }>([
          'talk',
          'Retained',
          message,
          '--timeout',
          '8',
        ])
      );
      expect(output.status).toBe('completed');
      expect(output.requestId).toMatch(/^req_[0-9a-f-]+$/);
      // The mock records logical nonblank lines, not the empty separator line.
      const payload = `[SYSTEM: ${preamble}]\n${message}`;
      await fixture.waitForEvent(
        (event) =>
          event.event === 'submitted' &&
          event.pid === peer.pid &&
          event.requestId === output.requestId &&
          event.message === payload
      );
      const events = fixture.events().filter((event) => event.requestId === output.requestId);
      expect(events.filter((event) => event.event === 'request')).toMatchObject([
        { pid: peer.pid, message: payload },
      ]);
      expect(events.filter((event) => event.event === 'submitted')).toMatchObject([
        { pid: peer.pid, message: payload },
      ]);
      expect(fixture.paneMetadata()).toBe(originalMetadata);
    });
    // Bounded CLI startups plus one response wait; no fixed sleeps or host tmux.
  }, 30_000);
});
