import {
  existsSync,
  readFileSync,
  realpathSync,
  readlinkSync,
  readdirSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { expectError, parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';
import { createArtifact } from '../support/native-artifact.js';
import { workspaceVersion } from '../support/workspace-version.js';
import {
  INSTALL_PROCESS_BUDGET_MS,
  installPrefix,
  install,
  currentPointer,
  receiptPath,
} from '../support/native-installation.js';

const officeVersion = workspaceVersion('tmt-office');

function officeSkill(): Buffer {
  return readFileSync(path.resolve('../extensions/tmt-office/skills/tmt-office/SKILL.md'));
}

function propCreateSkill(): Buffer {
  return readFileSync(path.resolve('../extensions/tmt-office/skills/tmt-prop-create/SKILL.md'));
}

function avatarCreateSkill(): Buffer {
  return readFileSync(path.resolve('../extensions/tmt-office/skills/tmt-avatar-create/SKILL.md'));
}

describe('native installation process contract', () => {
  it(
    'exposes explicit Office installation, local status and recoverable deactivation',
    { timeout: 60_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        sandbox.env.PATH = path.join(sandbox.root, 'no-provider-commands');
        const prefix = installPrefix(sandbox);
        const office = (args: string[]) =>
          runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
            deadlineMs: INSTALL_PROCESS_BUDGET_MS,
          });
        expectError(await office(['status']), 'OFFICE_NOT_INSTALLED');
        expectError(await office(['sync']), 'OFFICE_NOT_INSTALLED');
        expectError(await office([]), 'OFFICE_NOT_INSTALLED');
        expectError(await office(['install']), 'OFFICE_CONSENT_REQUIRED');
        expectError(await office(['uninstall']), 'OFFICE_CONSENT_REQUIRED');
        expectError(
          await office(['install', '--yes', '--archive', 'missing', '--manifest', 'missing']),
          'OFFICE_INSTALL_FAILED'
        );
        expect(existsSync(prefix)).toBe(false);
        const fixture = await createArtifact(sandbox, officeVersion, new Uint8Array(), 'office');
        const args = [
          'install',
          '--yes',
          '--archive',
          fixture.archive,
          '--manifest',
          fixture.manifest,
        ];
        const installed = await office(args);
        expect(installed.status, installed.stdout + installed.stderr).toBe(0);
        const installedDocument = parseWholeStdout(installed);
        expect(installedDocument).toMatchObject({
          installed: true,
          changed: true,
          version: officeVersion,
          skills: {
            installed: [
              {
                skill: 'tmt-avatar-create',
                target: path.join(sandbox.home, '.agents', 'skills', 'tmt-avatar-create'),
                changed: true,
              },
              {
                skill: 'tmt-office',
                target: path.join(sandbox.home, '.agents', 'skills', 'tmt-office'),
                changed: true,
              },
              {
                skill: 'tmt-prop-create',
                target: path.join(sandbox.home, '.agents', 'skills', 'tmt-prop-create'),
                changed: true,
              },
            ],
          },
        });
        const officeSkillTarget = path.join(sandbox.home, '.agents', 'skills', 'tmt-office');
        const propSkillTarget = path.join(sandbox.home, '.agents', 'skills', 'tmt-prop-create');
        const avatarSkillTarget = path.join(sandbox.home, '.agents', 'skills', 'tmt-avatar-create');
        expect(readFileSync(path.join(realpathSync(officeSkillTarget), 'SKILL.md'))).toEqual(
          officeSkill()
        );
        expect(readFileSync(path.join(realpathSync(propSkillTarget), 'SKILL.md'))).toEqual(
          propCreateSkill()
        );
        expect(readFileSync(path.join(realpathSync(avatarSkillTarget), 'SKILL.md'))).toEqual(
          avatarCreateSkill()
        );
        expect(existsSync(path.join(sandbox.home, '.agents', 'skills', 'tmux-team'))).toBe(false);
        expect(existsSync(path.join(sandbox.home, '.agents', 'skills', 'tmt-inbox'))).toBe(false);
        const status = await office(['status']);
        expect(status.status, status.stdout + status.stderr).toBe(0);
        expect(status.stderr).toBe('');
        expect(parseWholeStdout(status)).toMatchObject({
          installed: true,
          protocolVersion: '1',
          version: officeVersion,
          service: { running: false },
        });
        expect(parseWholeStdout(await office([]))).toEqual(parseWholeStdout(status));
        const bareHuman = await runCli(sandbox, ['office', '--prefix', prefix]);
        expect(bareHuman.status, bareHuman.stdout + bareHuman.stderr).toBe(0);
        expect(bareHuman.stderr).toBe('');
        expect(bareHuman.stdout).toContain('Local service is stopped.');
        expect(bareHuman.stdout).toContain('tmt office start');
        expect(bareHuman.stdout).toContain('tmt learn --skill tmt-office');
        const emptySync = await office(['sync']);
        expect(emptySync.status, emptySync.stdout).toBe(0);
        expect(emptySync.stderr).toBe('');
        expect(parseWholeStdout(emptySync)).toEqual({
          completed: 0,
          failed: 0,
          pending: 0,
          failureCode: null,
        });
        expect(existsSync(sandbox.database)).toBe(false);
        expect(existsSync(path.join(sandbox.globalDir, 'office'))).toBe(false);
        expect(parseWholeStdout(await office(args))).toMatchObject({
          changed: false,
          skills: {
            installed: [
              { skill: 'tmt-avatar-create', changed: false },
              { skill: 'tmt-office', changed: false },
              { skill: 'tmt-prop-create', changed: false },
            ],
          },
        });
        const payload = readFileSync(path.join(prefix, 'bin/tmt-office'));
        const releases = path.join(prefix, 'lib/tmt-office/releases');
        const release = readdirSync(releases)[0];
        // Interrupted removal may lose the command link before the activation.
        // Report that state honestly and allow explicit removal to finish.
        unlinkSync(path.join(prefix, 'bin/tmt-office'));
        expectError(await office(['status']), 'OFFICE_INSTALLATION_INVALID');
        expect(parseWholeStdout(await office(['uninstall', '--yes']))).toEqual({
          installed: false,
          changed: true,
          retainedReleases: true,
        });
        expect(readFileSync(path.join(officeSkillTarget, 'SKILL.md'))).toEqual(officeSkill());
        expect(readFileSync(path.join(propSkillTarget, 'SKILL.md'))).toEqual(propCreateSkill());
        expect(readFileSync(path.join(avatarSkillTarget, 'SKILL.md'))).toEqual(avatarCreateSkill());
        expect(readFileSync(path.join(releases, release, 'tmt-office')).equals(payload)).toBe(true);
        expectError(await office(['status']), 'OFFICE_NOT_INSTALLED');
        expect(parseWholeStdout(await office(['uninstall', '--yes']))).toMatchObject({
          changed: false,
        });
        unlinkSync(officeSkillTarget);
        writeFileSync(officeSkillTarget, 'user-owned Office skill');
        const partial = await office(args);
        expect(expectError(partial, 'OFFICE_SKILLS_FAILED')).toMatchObject({
          installed: true,
          changed: true,
          version: officeVersion,
          skills: {
            installed: [{ skill: 'tmt-avatar-create', changed: false }],
          },
        });
        expect(readFileSync(officeSkillTarget, 'utf8')).toBe('user-owned Office skill');
        expect(parseWholeStdout(await office(['status']))).toMatchObject({ installed: true });

        const recovered = parseWholeStdout(await office([...args, '--force']));
        expect(recovered).toMatchObject({
          changed: false,
          skills: {
            installed: [
              { skill: 'tmt-avatar-create', changed: false },
              { skill: 'tmt-office', changed: true },
              { skill: 'tmt-prop-create', changed: false },
            ],
          },
        });
        const backup = (
          recovered.skills as { installed: Array<{ skill: string; backup?: string }> }
        ).installed.find((item) => item.skill === 'tmt-office')?.backup;
        expect(backup).toBeDefined();
        expect(readFileSync(backup!, 'utf8')).toBe('user-owned Office skill');
        expect(readFileSync(path.join(officeSkillTarget, 'SKILL.md'))).toEqual(officeSkill());

        unlinkSync(officeSkillTarget);
        writeFileSync(officeSkillTarget, 'second user-owned Office skill');
        const plainForced = await runCli(
          sandbox,
          ['office', '--prefix', prefix, ...args, '--force'],
          { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
        );
        expect(plainForced.status, plainForced.stdout + plainForced.stderr).toBe(0);
        expect(plainForced.stderr).toBe('');
        expect(plainForced.stdout).toContain(
          `Installed shared skill 'tmt-office' at ${officeSkillTarget}\n`
        );
        const plainBackup = plainForced.stdout.match(/Recoverable backup: (.+)\n/)?.[1];
        expect(plainBackup).toBeDefined();
        expect(readFileSync(plainBackup!, 'utf8')).toBe('second user-owned Office skill');
        expect(existsSync(sandbox.database)).toBe(false);
      });
    }
  );

  it(
    'runs board edits and replies through the installed companion',
    { timeout: 240_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const prefix = installPrefix(sandbox);
        const fixture = await createArtifact(sandbox, officeVersion, new Uint8Array(), 'office');
        const office = (args: string[], outputLimitBytes = 1024 * 1024) =>
          runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
            deadlineMs: 30_000,
            outputLimitBytes,
          });
        const installed = await office([
          'install',
          '--yes',
          '--archive',
          fixture.archive,
          '--manifest',
          fixture.manifest,
        ]);
        expect(installed.status, installed.stdout + installed.stderr).toBe(0);
        expect((await runCli(sandbox, ['identity', 'create', 'Alice', '--json'])).status).toBe(0);
        const created = parseWholeStdout(
          await office([
            'board',
            'post',
            '--general',
            '--identity',
            'Alice',
            '--title',
            'before',
            '--body',
            'before',
          ])
        ) as { entryId: string; threadId: string };
        const edited = await office([
          'board',
          'edit',
          created.entryId,
          '--identity',
          'Alice',
          '--title',
          'after title',
          '--body',
          'after body',
          '--if-revision',
          '1',
        ]);
        expect(edited.status, edited.stdout + edited.stderr).toBe(0);
        const shown = parseWholeStdout(await office(['board', 'show', created.entryId])) as {
          thread: { title: string; body: string };
        };
        expect(shown.thread).toMatchObject({ title: 'after title', body: 'after body' });

        const replyBody = 'one durable reply';
        const reply = await office([
          'board',
          'reply',
          created.threadId,
          '--identity',
          'Alice',
          '--body',
          replyBody,
        ]);
        expect(reply.status, reply.stdout + reply.stderr).toBe(0);
        const page = await office(['board', 'show', created.threadId, '--reply-limit', '1']);
        expect(page.status, page.stdout + page.stderr).toBe(0);
        const document = parseWholeStdout(page) as {
          thread: { title: string; body: string };
          replies: { body: string }[];
        };
        expect(document.thread).toMatchObject({ title: 'after title', body: 'after body' });
        expect(document.replies.map((entry) => entry.body)).toEqual([replyBody]);
      });
    }
  );

  it(
    'installs Office explicitly without changing CLI bytes, receipts or application state',
    { timeout: 60_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const version = (await runCli(sandbox, ['--version'])).stdout.trim();
        const cli = await createArtifact(sandbox, version);
        const prefix = installPrefix(sandbox);
        const installedCli = await install(sandbox, cli, prefix);
        const cliReceipt = readFileSync(receiptPath(prefix));
        const pointer = readlinkSync(currentPointer(prefix));
        const office = await createArtifact(sandbox, officeVersion, new Uint8Array(), 'office');
        const installed = await install(sandbox, office, prefix, ['--product', 'office']);
        expect(installed).toEqual({
          executable: path.join(realpathSync(prefix), 'bin/tmt-office'),
          version: officeVersion,
          changed: true,
        });
        expect(readlinkSync(installed.executable)).toBe('../lib/tmt-office/current/tmt-office');
        expect(
          readFileSync(installed.executable).equals(
            readFileSync(path.resolve('../rust/target/debug/tmt-office'))
          )
        ).toBe(true);
        const probe = await runCli(
          { ...sandbox, cli: { executable: installed.executable, args: [] } },
          ['__tmt-office', '1', 'probe']
        );
        expect(probe.status).toBe(0);
        expect(probe.stderr).toBe('');
        expect(probe.stdout).toBe(`TMT-OFFICE/1\n${officeVersion}\n`);
        const rejected = await runCli(
          { ...sandbox, cli: { executable: installed.executable, args: [] } },
          ['__tmt-office', '2', 'probe']
        );
        expect(rejected.status).toBe(1);
        expect(rejected.stdout).toBe('');
        expect(rejected.stderr).toBe('Unsupported Office invocation or protocol version.\n');
        expect(await install(sandbox, office, prefix, ['--product', 'office'])).toEqual({
          ...installed,
          changed: false,
        });
        expect(readlinkSync(currentPointer(prefix))).toBe(pointer);
        expect(readFileSync(receiptPath(prefix)).equals(cliReceipt)).toBe(true);
        expect(
          readFileSync(installedCli.executable).equals(readFileSync(sandbox.cli.executable))
        ).toBe(true);
        expect(existsSync(sandbox.database)).toBe(false);
      });
    }
  );
});
