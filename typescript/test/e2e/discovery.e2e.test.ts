import fs from 'node:fs';
import path from 'node:path';
import { durableState } from './identity-state-oracle.js';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture } from './harness.js';

describe('outcome-aware CLI discovery on a private tmux server', () => {
  it('hints after a real temporary bind but not after repeats, promotion or JSON', async () => {
    await withE2EFixture(async (fixture) => {
      const first = await fixture.runCli(['name', 'First Agent']);
      expect(first.code).toBe(0);
      expect(first.stdout).toContain("Bound temporary identity 'First Agent'");
      expect(first.stderr).toBe(
        "hint: this temporary identity ends with its pane; keep it with tmt identity create -- 'First Agent'; use -s when binding\n"
      );

      const repeated = await fixture.runCli(['name', 'First Agent']);
      expect(repeated.code).toBe(0);
      expect(repeated.stdout).toEqual(first.stdout);
      expect(repeated.stderr).toBe('');

      const promoted = await fixture.runCli(['name', 'First Agent', '-s']);
      expect(promoted.code).toBe(0);
      expect(promoted.stdout).toContain("Bound saved identity 'First Agent'");
      expect(promoted.stderr).toBe('');

      const json = await fixture.runJsonCli<{ bound: boolean; lifetime: string }>(['whoami']);
      expect(json.code).toBe(0);
      expect(json.stderr).toBe('');
      expect(json.json).toMatchObject({ bound: true, lifetime: 'saved' });
    });
  });

  it('names a newly saved binding without offering to save it again', async () => {
    await withE2EFixture(async (fixture) => {
      const first = await fixture.runCli(['name', '-s', 'Saved Agent']);
      expect(first.code).toBe(0);
      expect(first.stdout).toContain("Bound saved identity 'Saved Agent'");
      expect(first.stderr).toBe(
        "hint: receive work for this saved identity with tmt x listen --identity='Saved Agent'\n"
      );
      const repeated = await fixture.runCli(['name', '-s', 'Saved Agent']);
      expect(repeated.code).toBe(0);
      expect(repeated.stderr).toBe('');
    });
  });
});

type Candidate = {
  pane: string;
  expect: string | null;
  adoptable: boolean;
  state: string;
  identity: null | { id: string; name: string };
  display: { title: string };
};
type Inventory = {
  version: number;
  candidates: Candidate[];
  sources: Array<{ status: string }>;
  truncated: boolean;
};

describe('read-only candidate inventory and guarded add', () => {
  it('adopts two unmarked panes in sequence with their expectations from one scan', async () => {
    await withE2EFixture(async (fixture) => {
      const firstPane = fixture.pane;
      const secondPane = (await fixture.createMockPane('second candidate')).pane;
      const serverId = () => fixture.tmux(['display-message', '-p', '#{@tmt.server-id}']).trim();
      expect(serverId()).toBe('');
      const scan = await fixture.runJsonCli<Inventory>([
        'discover',
        '--socket',
        fixture.socketPath,
      ]);
      expect(scan.code, scan.stderr).toBe(0);
      expect(scan.json!.candidates).toHaveLength(2);
      expect(serverId()).toBe('');
      const candidates = [firstPane, secondPane].map((pane) =>
        scan.json!.candidates.find((candidate) => candidate.pane === pane)!
      );
      const ids: string[] = [];
      for (const [index, candidate] of candidates.entries()) {
        expect(candidate.adoptable).toBe(true);
        expect(candidate.expect).toMatch(/^[0-9a-f]{64}$/);
        const bound = await fixture.runJsonCli<{ status: string; id: string }>([
          'add',
          candidate.pane,
          `batch worker ${index}`,
          '--socket',
          fixture.socketPath,
          '--expect',
          candidate.expect!,
          '-s',
        ]);
        expect(bound.code, bound.stderr).toBe(0);
        expect(bound.json!.status).toBe('bound');
        expect(serverId()).not.toBe('');
        ids.push(bound.json!.id);
      }
      expect(new Set(ids).size).toBe(2);
      const state = durableState(fixture);
      expect(state.identities).toHaveLength(2);
      expect(state.bindings).toHaveLength(2);
      for (const [index, candidate] of candidates.entries()) {
        expect(state.bindings).toContainEqual(
          expect.objectContaining({ identity_id: ids[index], pane_id: candidate.pane })
        );
      }
    });
  }, 30_000);

  it('refuses an old expectation after an unmarked server is replaced, then accepts a fresh one', async () => {
    await withE2EFixture(async (fixture) => {
      const firstPane = fixture.pane;
      const serverId = () => fixture.tmux(['display-message', '-p', '#{@tmt.server-id}']).trim();
      const scan = await fixture.runJsonCli<Inventory>([
        'discover',
        '--socket',
        fixture.socketPath,
      ]);
      expect(scan.code, scan.stderr).toBe(0);
      const original = scan.json!.candidates.find((candidate) => candidate.pane === firstPane)!;
      expect(original.adoptable).toBe(true);
      expect(original.expect).toMatch(/^[0-9a-f]{64}$/);
      expect(serverId()).toBe('');
      const oldServerPid = fixture.serverPid;
      const restarted = await fixture.restartServer();
      expect(fixture.serverPid).not.toBe(oldServerPid);
      // The address exists again: refusal must fence replacement, not absence.
      expect(restarted.pane).toBe(firstPane);
      expect(serverId()).toBe('');
      const rescan = await fixture.runJsonCli<Inventory>([
        'discover',
        '--socket',
        fixture.socketPath,
      ]);
      expect(rescan.code, rescan.stderr).toBe(0);
      const fresh = rescan.json!.candidates.find((candidate) => candidate.pane === firstPane)!;
      expect(fresh.adoptable).toBe(true);
      expect(fresh.expect).toMatch(/^[0-9a-f]{64}$/);
      expect(fresh.expect).not.toBe(original.expect);
      const database = path.join(fixture.globalDir, 'tmux-team.db');
      expect(fs.existsSync(database)).toBe(false);
      const refused = await fixture.runJsonCli<{ status: string; error: { code: string } }>([
        'add',
        restarted.pane,
        'replacement worker',
        '--socket',
        fixture.socketPath,
        '--expect',
        original.expect!,
        '-s',
      ]);
      expect(refused.code, refused.stderr).toBe(3);
      expect(refused.json).toMatchObject({ status: 'refused', error: { code: 'PANE_CHANGED' } });
      expect(serverId()).toBe('');
      expect(fixture.paneMetadata()).toBe('');
      expect(fs.existsSync(database)).toBe(false);
      const bound = await fixture.runJsonCli<{ status: string; id: string }>([
        'add',
        restarted.pane,
        'replacement worker',
        '--socket',
        fixture.socketPath,
        '--expect',
        fresh.expect!,
        '-s',
      ]);
      expect(bound.code, bound.stderr).toBe(0);
      expect(bound.json!.status).toBe('bound');
      expect(durableState(fixture).bindings).toEqual([
        expect.objectContaining({ identity_id: bound.json!.id, pane_id: restarted.pane }),
      ]);
    });
  }, 30_000);

  it('discovers an unmarked server, retains literal titles, and binds only an explicitly guarded pane', async () => {
    await withE2EFixture(async (fixture) => {
      // select-pane -T expands formats and rejects actual LF/TAB in titles.
      // Expanding an option once supplies literal format-like text and escapes.
      const requestedTitle = String.raw`literal #{run-shell:never} $(never)\nnew row\tstill one title`;
      fixture.tmux(['set-option', '-p', '-t', fixture.pane, '@discovery-title', requestedTitle]);
      fixture.tmux(['select-pane', '-t', fixture.pane, '-T', '#{@discovery-title}']);
      fixture.tmux(['set-option', '-p', '-t', fixture.pane, '-u', '@discovery-title']);
      // tmux versions differ in title backslash sanitization. Compare discovery
      // with the independently observed native value, retaining literal syntax.
      const title = fixture.paneTitle();
      expect(title).toContain('literal #{run-shell:never} $(never)');
      expect(title).toMatch(/\\+nnew row\\+tstill one title$/);
      const serverId = () => fixture.tmux(['display-message', '-p', '#{@tmt.server-id}']).trim();
      expect(serverId()).toBe('');
      const scan = await fixture.runJsonCli<Inventory>([
        'discover',
        '--socket',
        fixture.socketPath,
      ]);
      expect(scan.code, scan.stderr).toBe(0);
      expect(scan.json!.truncated).toBe(false);
      const candidate = scan.json!.candidates.find((c) => c.pane === fixture.pane)!;
      expect(candidate.display.title).toBe(title);
      expect(fixture.paneTitle()).toBe(title);
      expect(candidate.identity).toBe(null);
      expect(candidate.adoptable).toBe(true);
      expect(candidate.expect).toMatch(/^[0-9a-f]{64}$/);
      expect(serverId()).toBe('');
      expect(fs.existsSync(path.join(fixture.globalDir, 'tmux-team.db'))).toBe(false);
      const refused = await fixture.runJsonCli<{ status: string; error: { code: string } }>([
        'add',
        fixture.pane,
        'new identity',
        '--socket',
        fixture.socketPath,
        '--expect',
        '0'.repeat(64),
        '-s',
      ]);
      expect(refused.code).toBe(3);
      expect(refused.json).toMatchObject({ status: 'refused', error: { code: 'PANE_CHANGED' } });
      expect(serverId()).toBe('');
      expect(fs.existsSync(path.join(fixture.globalDir, 'tmux-team.db'))).toBe(false);
      const bound = await fixture.runJsonCli<{ id: string; status: string; lifetime: string }>([
        'add',
        fixture.pane,
        'new identity',
        '--socket',
        fixture.socketPath,
        '--expect',
        candidate.expect!,
        '-s',
      ]);
      expect(bound.code, bound.stderr).toBe(0);
      expect(bound.json).toMatchObject({ status: 'bound', lifetime: 'saved' });
      expect(serverId()).not.toBe('');
      const before = durableState(fixture);
      const rescan = await fixture.runJsonCli<Inventory>([
        'discover',
        '--socket',
        fixture.socketPath,
      ]);
      expect(rescan.code, rescan.stderr).toBe(0);
      expect(durableState(fixture)).toEqual(before);
      const known = rescan.json!.candidates.find((c) => c.pane === fixture.pane)!;
      expect(known).toMatchObject({ state: 'known', identity: { id: bound.json!.id } });
      const repeat = await fixture.runJsonCli<{ status: string; id: string }>([
        'add',
        fixture.pane,
        '--identity',
        bound.json!.id,
        '--socket',
        fixture.socketPath,
        '--expect',
        known.expect!,
      ]);
      expect(repeat.code, repeat.stderr).toBe(0);
      expect(repeat.json).toMatchObject({ status: 'already_bound', id: bound.json!.id });
      expect(durableState(fixture).bindings).toHaveLength(1);
    });
  });

  it('links by UUID across rename and refuses old expectations and canonical-name reuse without effects', async () => {
    await withE2EFixture(async (fixture) => {
      const created = await fixture.runJsonCli<{ identity: { id: string } }>([
        'identity',
        'create',
        'old name',
      ]);
      expect(created.code).toBe(0);
      expect((await fixture.runJsonCli(['mv', 'old name', 'new name'])).code).toBe(0);
      const scan = await fixture.runJsonCli<Inventory>([
        'discover',
        '--socket',
        fixture.socketPath,
      ]);
      const candidate = scan.json!.candidates.find((c) => c.pane === fixture.pane)!;
      const before = durableState(fixture);
      const refused = await fixture.runJsonCli<{ status: string; error: { code: string } }>([
        'add',
        fixture.pane,
        'NEW NAME',
        '--socket',
        fixture.socketPath,
        '--expect',
        candidate.expect!,
      ]);
      expect(refused.code).toBe(5);
      expect(refused.json).toMatchObject({
        status: 'refused',
        error: { code: 'NAME_ALREADY_ACTIVE' },
      });
      expect(durableState(fixture)).toEqual(before);
      const linked = await fixture.runJsonCli<{ id: string; name: string }>([
        'add',
        fixture.pane,
        '--identity',
        created.json!.identity.id,
        '--socket',
        fixture.socketPath,
        '--expect',
        candidate.expect!,
      ]);
      expect(linked.code, linked.stderr).toBe(0);
      expect(linked.json).toMatchObject({ id: created.json!.identity.id, name: 'new name' });
      const after = durableState(fixture);
      const stale = await fixture.runJsonCli<{ status: string; error: { code: string } }>([
        'add',
        fixture.pane,
        '--identity',
        created.json!.identity.id,
        '--socket',
        fixture.socketPath,
        '--expect',
        candidate.expect!,
      ]);
      expect(stale.code).toBe(3);
      expect(stale.json).toMatchObject({ status: 'refused', error: { code: 'PANE_CHANGED' } });
      expect(durableState(fixture)).toEqual(after);
    });
  });
});
