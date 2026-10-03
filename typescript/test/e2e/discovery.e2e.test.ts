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
