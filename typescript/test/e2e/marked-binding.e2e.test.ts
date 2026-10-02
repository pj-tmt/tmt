import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { durableState } from './identity-state-oracle.js';

interface Bound {
  bound: true;
  id: string;
  name: string;
  pane: string;
  lifetime: 'temporary' | 'saved';
}

function mark(fixture: E2EFixture, pane: string): void {
  fixture.tmux(['select-pane', '-t', pane, '-m']);
}

function markedPane(fixture: E2EFixture): string {
  return fixture.tmux(['list-panes', '-a', '-f', '#{pane_marked}', '-F', '#{pane_id}']).trim();
}

function badge(fixture: E2EFixture, pane: string): string {
  return fixture.tmux(['show-options', '-p', '-qv', '-t', pane, '@tmux-team.badge']).trim();
}

describe('explicit marked-pane binding', { concurrent: false }, () => {
  it('binds the marked pane without caller discovery and preserves repeat, save, conflicts, and the mark', async () => {
    await withE2EFixture(async (fixture) => {
      const peer = await fixture.createMockPane('marked-peer');
      mark(fixture, peer.pane);
      expect(markedPane(fixture)).toBe(peer.pane);
      expectJsonResult(
        await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'on', '--global'], {
          withoutTmux: true,
        })
      );

      const initial = expectJsonResult(
        await fixture.runJsonCli<Bound>(['marked', 'Reviewer'], { outsideTmux: true })
      );
      expect(initial).toMatchObject({
        bound: true,
        name: 'Reviewer',
        pane: peer.pane,
        lifetime: 'temporary',
      });
      expect(markedPane(fixture)).toBe(peer.pane);
      expect(fixture.paneMetadata()).toBe('');
      expect(badge(fixture, peer.pane)).toBe('Reviewer (tmt)');

      const saved = expectJsonResult(
        await fixture.runJsonCli<Bound>(['marked', 'Reviewer', '--save'], {
          caller: { pane: null },
        })
      );
      expect(saved).toMatchObject({ id: initial.id, pane: peer.pane, lifetime: 'saved' });
      expect(markedPane(fixture)).toBe(peer.pane);

      const occupied = await fixture.runJsonCli<{ error: { code: string } }>(['marked', 'Other']);
      expect(occupied.code).toBe(5);
      expect(occupied.json?.error.code).toBe('PANE_ALREADY_BOUND');
      expect(markedPane(fixture)).toBe(peer.pane);
      expect(badge(fixture, peer.pane)).toBe('Reviewer (tmt)');

      mark(fixture, fixture.pane);
      const activeName = await fixture.runJsonCli<{ error: { code: string } }>([
        'marked',
        'Reviewer',
      ]);
      expect(activeName.code).toBe(5);
      expect(activeName.json?.error.code).toBe('NAME_ALREADY_ACTIVE');
      expect(markedPane(fixture)).toBe(fixture.pane);
      expect(fixture.paneMetadata()).toBe('');
      expect(badge(fixture, fixture.pane)).toBe('');
    });
  });

  it('fails clearly for no mark, invalid names, and inaccessible selected servers without fallback', async () => {
    await withE2EFixture(async (fixture) => {
      const missing = await fixture.runJsonCli<{ error: { code: string; suggestion: string } }>([
        'marked',
        'Missing',
      ]);
      expect(missing.code).toBe(3);
      expect(missing.json?.error).toEqual({
        code: 'MARKED_PANE_NOT_FOUND',
        message: 'No marked pane was found on the selected tmux server.',
        suggestion: 'Mark the intended pane in tmux, then retry.',
      });
      expect(fs.existsSync(path.join(fixture.globalDir, 'tmux-team.db'))).toBe(false);

      mark(fixture, fixture.pane);
      const invalid = await fixture.runJsonCli<{ error: { code: string } }>([
        'marked',
        'bad\u007f',
      ]);
      expect(invalid.code).toBe(1);
      expect(invalid.json?.error.code).toBe('INVALID_NAME');
      const afterInvalid = durableState(fixture);
      expect(afterInvalid.identities).toEqual([]);
      expect(afterInvalid.bindings).toEqual([]);
      expect(markedPane(fixture)).toBe(fixture.pane);

      const inaccessible = await fixture.runJsonCli<{ error: { code: string } }>(
        ['marked', 'Elsewhere'],
        {
          caller: {
            tmux: `${fixture.socketPath}.missing,${fixture.serverPid},0`,
            pane: null,
          },
        }
      );
      expect(inaccessible.code).toBe(1);
      expect(inaccessible.json?.error.code).toBe('RECONCILIATION_FAILED');
      expect(durableState(fixture)).toEqual(afterInvalid);
      expect(fixture.paneMetadata()).toBe('');
    });
  });

  it('uses only the invocation-selected server when two private servers have marks', async () => {
    await withE2EFixture(async (first) => {
      await withE2EFixture(
        async (second) => {
          const firstPeer = await first.createMockPane('first-mark');
          const secondPeer = await second.createMockPane('second-mark');
          mark(first, firstPeer.pane);
          mark(second, secondPeer.pane);

          const local = expectJsonResult(
            await first.runJsonCli<Bound>(['marked', 'Local'], { caller: { pane: null } })
          );
          expect(local.pane).toBe(firstPeer.pane);
          expect(first.paneMetadata(firstPeer.pane)).not.toBe('');
          expect(second.paneMetadata(secondPeer.pane)).toBe('');
          expect(markedPane(first)).toBe(firstPeer.pane);
          expect(markedPane(second)).toBe(secondPeer.pane);
        },
        { globalDir: first.globalDir }
      );
    });
  });

  it('freezes the resolved target across mark changes and fails if that target disappears', async () => {
    await withE2EFixture(async (fixture) => {
      const first = await fixture.createMockPane('first-mark');
      const second = await fixture.createMockPane('second-mark');
      mark(fixture, first.pane);
      fixture.enableMetadataBarrier({ phase: 'before' });
      const binding = fixture.runCliProcess<Bound>(['--json', 'marked', 'Frozen']);
      await fixture.waitForMetadataBarrier();
      mark(fixture, second.pane);
      fixture.releaseMetadataBarrier();
      const result = expectJsonResult(await binding.result);
      expect(result.pane).toBe(first.pane);
      expect(markedPane(fixture)).toBe(second.pane);
      expect(fixture.paneMetadata(first.pane)).not.toBe('');
      expect(fixture.paneMetadata(second.pane)).toBe('');

      fixture.enableMetadataBarrier({ phase: 'before' });
      const vanishing = fixture.runCliProcess<{ error: { code: string } }>([
        '--json',
        'marked',
        'Vanished',
      ]);
      await fixture.waitForMetadataBarrier();
      fixture.tmux(['kill-pane', '-t', second.pane]);
      fixture.releaseMetadataBarrier();
      const failed = await vanishing.result;
      expect(failed.code).toBe(1);
      expect(failed.json?.error.code).toBe('RECONCILIATION_FAILED');
      expect(failed.stdout).not.toContain('"bound":true');
      expect(durableState(fixture).bindings).toHaveLength(1);
      expect(
        durableState(fixture).identities.find((identity) => identity.canonical_name === 'vanished')
      ).toMatchObject({ retired_at_ms: null });
    });
  });
});
