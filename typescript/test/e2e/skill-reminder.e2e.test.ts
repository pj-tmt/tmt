import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture } from './harness.js';
import { releaseRealTmuxCli, spawnRealTmuxCli } from './real-tmux-caller.js';

describe('native passive skill guidance', () => {
  it('warns only on an eligible terminal command without mutating stale guidance or storage', async () => {
    await withE2EFixture(async (fixture) => {
      const home = fixture.createWorkspace('isolated-provider-home');
      for (const [key, value] of Object.entries({
        HOME: home,
        CODEX_HOME: path.join(home, '.codex'),
        PI_CODING_AGENT_DIR: path.join(home, '.pi'),
        OPENCODE_CONFIG_DIR: path.join(home, '.opencode'),
        XDG_CONFIG_HOME: path.join(home, '.config'),
      })) {
        fixture.tmux(['set-environment', '-g', key, value]);
      }
      const stale = path.join(home, '.claude', 'skills', 'tmux-team');
      fs.mkdirSync(stale, { recursive: true });
      const staleFile = path.join(stale, 'SKILL.md');
      fs.writeFileSync(staleFile, 'user-maintained stale guidance');

      for (const scenario of [
        { name: 'eligible', args: ['config', 'show'], terminal: true, json: false, warn: true },
        { name: 'machine', args: ['config', 'show'], terminal: true, json: true, warn: false },
        { name: 'redirected', args: ['config', 'show'], terminal: false, json: false, warn: false },
        { name: 'help', args: ['help'], terminal: true, json: false, warn: false },
      ]) {
        const process = await spawnRealTmuxCli(fixture, scenario.args, scenario);
        await releaseRealTmuxCli(fixture, process);
        expect(fs.readFileSync(process.exitPath, 'utf8')).toBe('0');
        const output = scenario.terminal
          ? fixture.tmux(['capture-pane', '-p', '-J', '-S', '-', '-t', process.pane])
          : fs.readFileSync(process.outputPath, 'utf8') +
            fs.readFileSync(process.errorPath, 'utf8');
        if (scenario.warn) {
          expect(output).toContain(
            `warning: Skill guidance needs inspection at ${stale} (1 location(s))`
          );
          expect(output).toContain(
            'hint: run tmt install for the intended provider; inspect conflicts before using --force, then reload the agent'
          );
        } else {
          expect(output).not.toContain('Skill guidance needs inspection');
        }
        if (scenario.json) {
          expect(JSON.parse(output.trim())).toMatchObject({
            resolved: { ui: { paneBadge: 'off' } },
          });
        } else if (scenario.name !== 'help') {
          expect(output).toContain('SETTINGS\n');
        }
        expect(fs.readFileSync(staleFile, 'utf8')).toBe('user-maintained stale guidance');
        expect(fs.existsSync(path.join(fixture.globalDir, 'tmux-team.db'))).toBe(false);
      }
    });
  }, 15_000);
});
