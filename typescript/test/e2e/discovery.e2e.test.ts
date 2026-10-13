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
  it('discovers an unmarked server, retains literal titles, and binds only an explicitly guarded pane', async () => {
    await withE2EFixture(async (fixture) => {
      const title = 'literal #{run-shell:never} $(never)\nnew row\tstill one title';
      fixture.tmux(['select-pane', '-t', fixture.pane, '-T', title]);
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
