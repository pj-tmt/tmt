import {
  chmodSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readlinkSync,
  readdirSync,
  realpathSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  expectError,
  parseWholeStdout,
  runCli,
  withSandbox,
  type Sandbox,
} from '../support/cli-process.js';
import { createArtifact, type ArtifactFixture } from '../support/native-artifact.js';
import { workspaceVersion } from '../support/workspace-version.js';

const officeVersion = workspaceVersion('tmt-office');

// Debug payload hashing/decompression and fsync are installation work, not the
// ordinary command-startup budget. Keep a separate finite process deadline.
const INSTALL_PROCESS_BUDGET_MS = 15_000;

type InstallResult = {
  readonly executable: string;
  readonly version: string;
  readonly changed: boolean;
};

function installPrefix(sandbox: Sandbox): string {
  return path.join(sandbox.root, 'native install prefix with spaces');
}

function officeSkill(): Buffer {
  return readFileSync(path.resolve('../extensions/tmt-office/skills/tmt-office/SKILL.md'));
}

function propCreateSkill(): Buffer {
  return readFileSync(path.resolve('../extensions/tmt-office/skills/tmt-prop-create/SKILL.md'));
}

function avatarCreateSkill(): Buffer {
  return readFileSync(path.resolve('../extensions/tmt-office/skills/tmt-avatar-create/SKILL.md'));
}

async function install(
  sandbox: Sandbox,
  fixture: ArtifactFixture,
  prefix: string,
  flags: readonly string[] = [],
  tmuxTeamHome?: string
): Promise<InstallResult> {
  const selected =
    tmuxTeamHome === undefined
      ? sandbox
      : {
          ...sandbox,
          cli: {
            executable: '/usr/bin/env',
            args: [`TMUX_TEAM_HOME=${tmuxTeamHome}`, sandbox.cli.executable, ...sandbox.cli.args],
          },
        };
  const result = await runCli(
    selected,
    [
      '__native-install',
      '--archive',
      fixture.archive,
      '--manifest',
      fixture.manifest,
      '--prefix',
      prefix,
      '--channel',
      'alpha',
      ...flags,
      '--json',
    ],
    { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
  );
  expect(result.status, result.stdout + result.stderr).toBe(0);
  expect(result.stderr).toBe('');
  return parseWholeStdout(result) as unknown as InstallResult;
}

function currentPointer(prefix: string): string {
  return path.join(prefix, 'lib', 'tmux-team', 'current');
}

function receiptPath(prefix: string): string {
  return path.join(currentPointer(prefix), 'receipt.json');
}

function currentReleaseId(prefix: string): string {
  return readlinkSync(currentPointer(prefix)).replace(/^releases\//, '');
}

function releaseIds(prefix: string): string[] {
  return readdirSync(path.join(prefix, 'lib', 'tmux-team', 'releases')).sort();
}

function receipt(prefix: string): Record<string, unknown> {
  return JSON.parse(readFileSync(receiptPath(prefix), 'utf8')) as Record<string, unknown>;
}

function artifactChecksum(fixture: ArtifactFixture): string {
  const manifest = JSON.parse(readFileSync(fixture.manifest, 'utf8')) as {
    artifacts: Record<string, { checksums: { sha256: string } }>;
  };
  return manifest.artifacts[path.basename(fixture.archive)].checksums.sha256;
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

  it('retains a generated operation ID across a companion storage uncertainty and replay', async () => {
    await withSandbox(async (sandbox) => {
      const prefix = installPrefix(sandbox);
      const companion = path.join(sandbox.root, 'storage-uncertain-office');
      writeFileSync(
        companion,
        `#!/bin/sh
control="\${0%/lib/tmt-office/releases/*}/storage-uncertain-operation"
case "$3" in
probe) printf 'TMT-OFFICE/1\\n0.1.0-alpha.2\\n' ;;
capabilities) printf 'TMT-OFFICE-CAPABILITIES/1\\noffice_board_v1\\n' ;;
board-post)
  input="$(cat)"
  operation_id="$(printf '%s' "$input" | sed -n 's/.*"operationId":"\\([^"]*\\)".*/\\1/p')"
  if [ ! -f "$control" ]; then
    printf '%s' "$operation_id" > "$control"
    printf '{"error":"STORAGE_ERROR"}'
  elif [ "$(cat "$control")" = "$operation_id" ]; then
    printf '{"entryId":"11111111-1111-4111-8111-111111111111","threadId":"11111111-1111-4111-8111-111111111111","revision":1,"created":true,"operationId":"%s"}' "$operation_id"
  else
    printf '{"error":"BOARD_IDEMPOTENCY_CONFLICT"}'
  fi
  ;;
*) exit 1 ;;
esac
`
      );
      chmodSync(companion, 0o755);
      const fixture = await createArtifact(
        sandbox,
        '0.1.0-alpha.2',
        new Uint8Array(),
        'office',
        companion
      );
      const office = (args: string[]) =>
        runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
          deadlineMs: 30_000,
        });
      expect(
        (
          await office([
            'install',
            '--yes',
            '--archive',
            fixture.archive,
            '--manifest',
            fixture.manifest,
          ])
        ).status
      ).toBe(0);

      const mutation = [
        'board',
        'post',
        '--general',
        '--owner',
        '--title',
        'possibly committed',
        '--body',
        'exact body',
      ];
      const firstAttempt = await office(mutation);
      expect(firstAttempt.status).toBe(1);
      const uncertain = expectError(firstAttempt, 'OFFICE_LOCAL_UNCERTAIN');
      const message = (uncertain.error as { message: string }).message;
      const match = message.match(
        /--operation-id ([0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})\.$/
      );
      expect(match).not.toBeNull();
      const operationId = match![1];
      expect(readFileSync(path.join(prefix, 'storage-uncertain-operation'), 'utf8')).toBe(
        operationId
      );

      const replay = parseWholeStdout(await office([...mutation, '--operation-id', operationId]));
      expect(replay).toEqual({
        entryId: '11111111-1111-4111-8111-111111111111',
        threadId: '11111111-1111-4111-8111-111111111111',
        revision: 1,
        created: true,
        operationId,
      });
      expect(parseWholeStdout(await office([...mutation, '--operation-id', operationId]))).toEqual(
        replay
      );
    });
  });

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

  it(
    'installs a real copied native executable from spaced paths and runs it with an empty PATH',
    { timeout: 60_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const versionResult = await runCli(sandbox, ['--version']);
        expect(versionResult.status).toBe(0);
        expect(versionResult.stderr).toBe('');
        const version = versionResult.stdout.trim();
        expect(version).toMatch(/^\d+\.\d+\.\d+-alpha(?:\.[0-9A-Za-z-]+)?$/);
        const fixture = await createArtifact(sandbox, version);
        const prefix = installPrefix(sandbox);
        const installed = await install(sandbox, fixture, prefix);
        expect(installed).toEqual({
          executable: path.join(realpathSync(prefix), 'bin', 'tmt'),
          version,
          changed: true,
        });
        expect(readlinkSync(installed.executable)).toBe('../lib/tmux-team/current/tmt');
        expect(existsSync(sandbox.database)).toBe(false);

        sandbox.env.PATH = '';
        const moved = await runCli(
          { ...sandbox, cli: { executable: installed.executable, args: [] } },
          ['--version']
        );
        expect(moved.status).toBe(0);
        expect(moved.stdout).toBe(`${version}\n`);
        expect(moved.stderr).toBe('');
        expect(existsSync(sandbox.database)).toBe(false);
      });
    }
  );

  it(
    'repeats as a no-op while application-state selectors change',
    { timeout: 60_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const versionResult = await runCli(sandbox, ['--version']);
        const fixture = await createArtifact(sandbox, versionResult.stdout.trim());
        const prefix = installPrefix(sandbox);
        const first = await install(sandbox, fixture, prefix);
        const pointer = readlinkSync(currentPointer(prefix));
        const before = readFileSync(path.join(prefix, 'lib', 'tmux-team', pointer, 'tmt'));

        const firstHome = path.join(sandbox.root, 'unrelated tmux state one');
        const second = await install(sandbox, fixture, prefix, [], firstHome);
        const secondHome = path.join(sandbox.root, 'unrelated tmux state two');
        const third = await install(sandbox, fixture, prefix, [], secondHome);

        expect(first.changed).toBe(true);
        expect(second).toEqual({ ...first, changed: false });
        expect(third).toEqual({ ...first, changed: false });
        expect(readlinkSync(currentPointer(prefix))).toBe(pointer);
        expect(
          readFileSync(path.join(prefix, 'lib', 'tmux-team', pointer, 'tmt')).equals(before)
        ).toBe(true);
        expect(releaseIds(prefix)).toHaveLength(1);
        expect(existsSync(firstHome)).toBe(false);
        expect(existsSync(secondHome)).toBe(false);
        expect(existsSync(sandbox.database)).toBe(false);
      });
    }
  );

  it(
    'records pin and unpin transitions while retaining the old release bytes',
    { timeout: 60_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const versionResult = await runCli(sandbox, ['--version']);
        const fixture = await createArtifact(sandbox, versionResult.stdout.trim());
        const prefix = installPrefix(sandbox);
        await install(sandbox, fixture, prefix);
        const originalId = currentReleaseId(prefix);
        const originalBytes = readFileSync(
          path.join(prefix, 'lib', 'tmux-team', 'releases', originalId, 'tmt')
        );

        const pinned = await install(sandbox, fixture, prefix, ['--pin']);
        const pinnedId = currentReleaseId(prefix);
        expect(pinned.changed).toBe(true);
        expect(pinnedId).not.toBe(originalId);
        expect(receipt(prefix)).toMatchObject({
          version: fixture.version,
          channel: 'alpha',
          pinned_version: fixture.version,
        });
        const managed = {
          ...sandbox,
          cli: { executable: pinned.executable, args: [] },
          env: { ...sandbox.env, PATH: path.dirname(pinned.executable) },
        };
        const pinnedReceipt = readFileSync(receiptPath(prefix));
        for (const command of ['upgrade', 'update']) {
          const result = await runCli(managed, [command, '--json']);
          expect(result.status).toBe(0);
          expect(result.stderr).toBe('');
          expect(parseWholeStdout(result)).toMatchObject({
            version: fixture.version,
            changed: false,
            channel: 'alpha',
            pinned: true,
            pinnedVersion: fixture.version,
            skippedPinned: true,
            products: [
              { product: 'cli', status: 'skippedPinned', version: fixture.version, error: null },
            ],
            skills: null,
            pathWarning: null,
          });
        }
        const shadowed = await runCli({ ...managed, env: { ...managed.env, PATH: '' } }, [
          'upgrade',
          '--json',
        ]);
        expect(shadowed.status).toBe(0);
        expect(parseWholeStdout(shadowed).pathWarning).toContain('PATH does not select');
        const badChannel = await runCli(managed, ['upgrade', '--channel', 'stable', '--json']);
        expect(badChannel.status).toBe(1);
        expectError(
          badChannel,
          'NATIVE_UPGRADE_FAILED',
          'The installation is pinned; explicitly select a version or unpin it.'
        );
        expect(readFileSync(receiptPath(prefix))).toEqual(pinnedReceipt);
        expect(existsSync(sandbox.database)).toBe(false);
        expect(
          readFileSync(path.join(prefix, 'lib', 'tmux-team', 'releases', originalId, 'tmt')).equals(
            originalBytes
          )
        ).toBe(true);

        const unpinned = await install(sandbox, fixture, prefix, ['--unpin']);
        const unpinnedId = currentReleaseId(prefix);
        expect(unpinned.changed).toBe(true);
        const stale = await runCli(
          {
            ...sandbox,
            cli: {
              executable: path.join(prefix, 'lib', 'tmux-team', 'releases', pinnedId, 'tmt'),
              args: [],
            },
          },
          ['upgrade', '--json']
        );
        expect(stale.status).toBe(1);
        expectError(
          stale,
          'NATIVE_UPGRADE_FAILED',
          'This executable is not the active managed release. Run the current native installation, or update using its original package manager.'
        );
        expect(currentReleaseId(prefix)).toBe(unpinnedId);
        expect(unpinnedId).not.toBe(pinnedId);
        expect(receipt(prefix)).toMatchObject({
          version: fixture.version,
          channel: 'alpha',
          pinned_version: null,
        });
        expect(releaseIds(prefix)).toEqual([originalId, pinnedId, unpinnedId].sort());
        expect(
          readFileSync(path.join(prefix, 'lib', 'tmux-team', 'releases', originalId, 'tmt')).equals(
            originalBytes
          )
        ).toBe(true);
        expect(existsSync(sandbox.database)).toBe(false);
      });
    }
  );

  it(
    'rejects an equal-version artifact with changed file bytes even when its archive digest is forged into the receipt',
    { timeout: 60_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const versionResult = await runCli(sandbox, ['--version']);
        const version = versionResult.stdout.trim();
        const originalFixture = await createArtifact(sandbox, version);
        const prefix = installPrefix(sandbox);
        await install(sandbox, originalFixture, prefix);
        const pointer = readlinkSync(currentPointer(prefix));
        const originalExecutable = readFileSync(
          path.join(prefix, 'lib', 'tmux-team', pointer, 'tmt')
        );
        const originalReceipt = receipt(prefix);
        const candidate = await createArtifact(
          sandbox,
          version,
          Buffer.from('\nintentionally different candidate bytes\n')
        );
        const forgedReceipt = {
          ...originalReceipt,
          archive_sha256: artifactChecksum(candidate),
        };
        const forgedReceiptBytes = Buffer.from(`${JSON.stringify(forgedReceipt)}\n`);
        writeFileSync(receiptPath(prefix), forgedReceiptBytes);

        // This candidate is intentionally never executed; it only tests equal-version file integrity.
        const result = await runCli(
          sandbox,
          [
            '__native-install',
            '--archive',
            candidate.archive,
            '--manifest',
            candidate.manifest,
            '--prefix',
            prefix,
            '--channel',
            'alpha',
            '--json',
          ],
          { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
        );
        expect(result.status).toBe(1);
        expectError(
          result,
          'NATIVE_INSTALL_FAILED',
          'Installed target or equal-version artifact integrity does not match.'
        );
        expect(readlinkSync(currentPointer(prefix))).toBe(pointer);
        expect(
          readFileSync(path.join(prefix, 'lib', 'tmux-team', pointer, 'tmt')).equals(
            originalExecutable
          )
        ).toBe(true);
        expect(readFileSync(receiptPath(prefix))).toEqual(forgedReceiptBytes);
        expect(receipt(prefix).file_sha256).toEqual(originalReceipt.file_sha256);
        expect(existsSync(sandbox.database)).toBe(false);
      });
    }
  );

  it(
    'refuses unmanaged collisions and a tampered receipt without changing owned bytes or pointer',
    { timeout: 60_000 },
    async () => {
      await withSandbox(async (sandbox) => {
        const versionResult = await runCli(sandbox, ['--version']);
        const fixture = await createArtifact(sandbox, versionResult.stdout.trim());
        const prefix = installPrefix(sandbox);
        mkdirSync(path.join(prefix, 'bin'), { recursive: true });
        const collision = path.join(prefix, 'bin', 'tmt');
        const collisionBytes = Buffer.from('user-owned command\n');
        writeFileSync(collision, collisionBytes);
        const collisionResult = await runCli(
          sandbox,
          [
            '__native-install',
            '--archive',
            fixture.archive,
            '--manifest',
            fixture.manifest,
            '--prefix',
            prefix,
            '--channel',
            'alpha',
            '--json',
          ],
          { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
        );
        expect(collisionResult.status).toBe(1);
        expectError(collisionResult, 'NATIVE_INSTALL_FAILED');
        expect(readFileSync(collision)).toEqual(collisionBytes);
        expect(existsSync(currentPointer(prefix))).toBe(false);

        const ownedPrefix = path.join(sandbox.root, 'owned prefix');
        await install(sandbox, fixture, ownedPrefix);
        const pointer = readlinkSync(currentPointer(ownedPrefix));
        const executable = readFileSync(path.join(ownedPrefix, 'lib', 'tmux-team', pointer, 'tmt'));
        writeFileSync(receiptPath(ownedPrefix), '{ malformed receipt');
        const tampered = await runCli(
          sandbox,
          [
            '__native-install',
            '--archive',
            fixture.archive,
            '--manifest',
            fixture.manifest,
            '--prefix',
            ownedPrefix,
            '--channel',
            'alpha',
            '--json',
          ],
          { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
        );
        expect(tampered.status).toBe(1);
        expectError(tampered, 'NATIVE_INSTALL_FAILED');
        expect(readlinkSync(currentPointer(ownedPrefix))).toBe(pointer);
        expect(
          readFileSync(path.join(ownedPrefix, 'lib', 'tmux-team', pointer, 'tmt')).equals(
            executable
          )
        ).toBe(true);
        expect(existsSync(sandbox.database)).toBe(false);
      });
    }
  );

  it('keeps the internal installer out of help and completion output', async () => {
    await withSandbox(async (sandbox) => {
      const help = await runCli(sandbox, ['help']);
      expect(help.status).toBe(0);
      expect(help.stdout).not.toContain('__native-install');
      expect(help.stdout).not.toContain('--archive');
      for (const shell of ['bash', 'zsh']) {
        const completion = await runCli(sandbox, ['completion', shell]);
        expect(completion.status).toBe(0);
        expect(completion.stdout).not.toContain('__native-install');
        expect(completion.stdout).not.toContain('--product');
        // Offline Office installation is public; only the internal publisher is hidden.
        expect(completion.stdout).toContain('--archive');
      }
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });
});
