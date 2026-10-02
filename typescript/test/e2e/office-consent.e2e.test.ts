import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { withE2EFixture } from './harness.js';
import { releaseRealTmuxCli, spawnRealTmuxCli } from './real-tmux-caller.js';

describe('optional Office consent on a real terminal', () => {
  it('inspects bare Office without prompting or creating an installation', async () => {
    await withE2EFixture(async (fixture) => {
      const prefix = path.join(fixture.root, 'optional office');
      const process = await spawnRealTmuxCli(fixture, ['office', '--prefix', prefix], {
        name: 'office-inspection',
        json: false,
        terminal: true,
      });
      await releaseRealTmuxCli(fixture, process);
      expect(fs.readFileSync(process.exitPath, 'utf8')).toBe('1');
      const output = fixture.tmux(['capture-pane', '-p', '-J', '-S', '-', '-t', process.pane]);
      expect(output).toContain('Install Office with: tmt office install --yes');
      expect(output).not.toContain('[y/N]');
      expect(fs.existsSync(prefix)).toBe(false);
    });
  });
});
