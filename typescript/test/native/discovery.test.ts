import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';

describe('first-time CLI discovery', () => {
  it('selects a saved-identity hint once for a real non-TTY creation, with opt-out and JSON isolation', async () => {
    await withSandbox(async (sandbox) => {
      const created = await runCli(sandbox, ['identity', 'create', 'Team Lead']);
      expect(created.status).toBe(0);
      expect(created.stdout).toContain("Created saved identity 'Team Lead'");
      expect(created.stderr).toBe(
        'Hint: To receive work for this saved identity, run `tmt x listen --identity <name>`.\n'
      );

      const repeat = await runCli(sandbox, ['identity', 'create', 'Team Lead']);
      expect(repeat.status).toBe(0);
      expect(repeat.stdout).toContain("Already exists: saved identity 'Team Lead'");
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
      expect(failed.stderr).not.toContain('Hint:');
    });
  });

  it('keeps offline Inbox acceptance available with hints disabled', async () => {
    await withSandbox(async (sandbox) => {
      const name = "Team's Lead";
      expect((await runCli(sandbox, ['identity', 'create', name])).status).toBe(0);
      sandbox.env.TMT_HINTS = 'off';
      const queued = await runCli(sandbox, ['talk', name, 'do not deliver']);
      expect(queued.status).toBe(0);
      expect(queued.stdout).toContain(`${name} is offline; the request is kept in Inbox`);
      expect(queued.stderr).toBe('');

      const unknown = await runCli(sandbox, ['talk', 'Missing', 'do not deliver']);
      expect(unknown.status).toBe(3);
      expect(unknown.stderr).not.toContain('--inbox');
    });
  });

  it('keeps the bare Office entry inspection-only when no companion is installed', async () => {
    await withSandbox(async (sandbox) => {
      const bare = await runCli(sandbox, ['office']);
      const status = await runCli(sandbox, ['office', 'status']);
      expect(bare.status).toBe(1);
      expect(bare).toEqual(status);
      expect(bare.stderr).toContain('Install Office with: tmt office install --yes');
      expect(existsSync(sandbox.database)).toBe(false);
      expect(existsSync(`${sandbox.globalDir}/office`)).toBe(false);

      const machine = await runCli(sandbox, ['--json', 'office']);
      const machineStatus = await runCli(sandbox, ['--json', 'office', 'status']);
      expect(machine).toEqual(machineStatus);
      expect(machine.stderr).toBe('');
      expect(parseWholeStdout(machine)).toMatchObject({
        error: { code: 'OFFICE_NOT_INSTALLED' },
      });
    });
  });

  it('offers the optional local Office path in learn while preserving exact skill output', async () => {
    await withSandbox(async (sandbox) => {
      const learn = await runCli(sandbox, ['learn']);
      expect(learn.status).toBe(0);
      expect(learn.stderr).toBe('');
      expect(learn.stdout).toContain('Optional local Office: inspect with tmt office');
      expect(learn.stdout).toContain('tmt learn --skill tmt-office');

      const skill = await runCli(sandbox, ['learn', '--skill', 'tmt-office']);
      expect(skill.status).toBe(0);
      expect(skill.stderr).toBe('');
      expect(skill.stdout).toBe(
        readFileSync(path.resolve('../skills/tmt-office/SKILL.md'), 'utf8')
      );
    });
  });
});
