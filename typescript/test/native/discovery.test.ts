import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';

describe('first-time CLI discovery', () => {
  it('selects a saved-identity hint once for a real non-TTY creation, with opt-out and JSON isolation', async () => {
    await withSandbox(async (sandbox) => {
      const created = await runCli(sandbox, ['identity', 'create', 'Team Lead']);
      expect(created.status).toBe(0);
      expect(created.stdout).toContain("Created saved identity 'Team Lead'");
      expect(created.stderr).toBe(
        "hint: receive work for this saved identity with tmt x listen --identity='Team Lead'\n"
      );

      const repeat = await runCli(sandbox, ['identity', 'create', 'Team Lead']);
      expect(repeat.status).toBe(0);
      expect(repeat.stdout).toContain("Saved identity 'Team Lead' already exists");
      expect(repeat.stderr).toBe('');

      const machine = await runCli(sandbox, ['--json', 'identity', 'create', 'Machine']);
      expect(machine.status).toBe(0);
      expect(machine.stderr).toBe('');
      expect(parseWholeStdout(machine)).toMatchObject({ created: true });

      sandbox.env.TMT_HINTS = 'off';
      const optedOut = await runCli(sandbox, ['identity', 'create', 'No Hint']);
      expect(optedOut.status).toBe(0);
      expect(optedOut.stderr).toBe('');
      expect(optedOut.stdout).toContain("Created saved identity 'No Hint'");

      const failed = await runCli(sandbox, ['identity', 'create', '   ']);
      expect(failed.status).not.toBe(0);
      expect(failed.stderr).not.toContain('hint: receive work');
    });
  });

  it('keeps offline Inbox acceptance available with hints disabled', async () => {
    await withSandbox(async (sandbox) => {
      const name = "Team's Lead";
      expect((await runCli(sandbox, ['identity', 'create', name])).status).toBe(0);
      sandbox.env.TMT_HINTS = 'off';
      const queued = await runCli(sandbox, ['talk', name, 'do not deliver', '--detach']);
      expect(queued.status).toBe(0);
      expect(queued.stdout).toContain(`for ${name} without a live endpoint`);
      expect(queued.stdout).toContain('tmt inbox --identity');
      expect(queued.stdout).toContain('tmt x show');
      expect(queued.stdout).not.toContain('tmt resume');
      expect(queued.stderr).toBe('');

      const unknown = await runCli(sandbox, ['talk', 'Missing', 'do not deliver']);
      expect(unknown.status).toBe(3);
      expect(unknown.stderr).not.toContain('--inbox');
    });
  });

  it('rejects retired Office commands without creating state when no external executable is installed', async () => {
    await withSandbox(async (sandbox) => {
      const bare = await runCli(sandbox, ['office']);
      const status = await runCli(sandbox, ['office', 'status']);
      expect(bare.status).toBe(1);
      expect(status.status).toBe(1);
      expect(bare.stderr).toContain("unrecognized subcommand 'office'");
      expect(status.stderr).toContain("unrecognized subcommand 'office'");
      expect(existsSync(sandbox.database)).toBe(false);
      expect(existsSync(`${sandbox.globalDir}/office`)).toBe(false);

      const machine = await runCli(sandbox, ['--json', 'office']);
      const machineStatus = await runCli(sandbox, ['--json', 'office', 'status']);
      expect(machineStatus.status).toBe(1);
      expect(machine.stderr).toBe('');
      expect(parseWholeStdout(machine)).toMatchObject({
        error: { code: 'USAGE_ERROR' },
      });
    });
  });

  it('offers current Core guidance and excludes retired Office guidance', async () => {
    await withSandbox(async (sandbox) => {
      const learn = await runCli(sandbox, ['learn']);
      expect(learn.status).toBe(0);
      expect(learn.stderr).toBe('');
      expect(learn.stdout).toContain('Read tmt learn --skill');
      expect(learn.stdout).not.toContain('tmt office');
      expect(learn.stdout).not.toContain('tmt-office');

      const skill = await runCli(sandbox, ['learn', '--skill']);
      expect(skill.status).toBe(0);
      expect(skill.stderr).toBe('');
      expect(skill.stdout).toBe(readFileSync(path.resolve('../skills/tmt/SKILL.md'), 'utf8'));
      const retired = await runCli(sandbox, ['learn', '--skill', 'tmt-office']);
      expect(retired.status).toBe(1);
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });
});
