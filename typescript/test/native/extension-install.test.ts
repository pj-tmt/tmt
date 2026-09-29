import {
  existsSync,
  lstatSync,
  readFileSync,
  readlinkSync,
  realpathSync,
  writeFileSync,
  chmodSync,
  mkdirSync,
} from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { expectError, parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';
import { createArtifact } from '../support/native-artifact.js';

const INSTALL_PROCESS_BUDGET_MS = 15_000;

describe('tmt extension install surface', () => {
  it('installs, lists offline, refuses without consent, and uninstalls squad keeping its releases', async () => {
    await withSandbox(async (sandbox) => {
      const prefix = path.join(sandbox.root, 'extension prefix');
      const squad = await createArtifact(sandbox, '0.1.0-alpha.1', new Uint8Array(), 'squad');
      const cli = (args: string[]) =>
        runCli(sandbox, [...args, '--json'], { deadlineMs: INSTALL_PROCESS_BUDGET_MS });

      // Consent is never assumed in a non-interactive run.
      const refused = await cli([
        'extension',
        'install',
        'squad',
        '--archive',
        squad.archive,
        '--manifest',
        squad.manifest,
        '--prefix',
        prefix,
      ]);
      expectError(refused, 'EXTENSION_CONSENT_REQUIRED');
      expect(existsSync(prefix)).toBe(false);
      expectError(await cli(['extension', 'install', 'cli', '--yes']), 'EXTENSION_UNKNOWN');

      const install = [
        'extension',
        'install',
        'squad',
        '--yes',
        '--archive',
        squad.archive,
        '--manifest',
        squad.manifest,
        '--prefix',
        prefix,
        '--channel',
        'alpha',
      ];
      const installed = await cli(install);
      expect(installed.stderr).toBe('');
      expect(parseWholeStdout(installed)).toEqual({
        extension: 'squad',
        installed: true,
        changed: true,
        version: '0.1.0-alpha.1',
        executable: path.join(realpathSync(prefix), 'bin/tmt-squad'),
      });
      for (const link of ['tmt-squad', 'tmt-sq'])
        expect(readlinkSync(path.join(prefix, 'bin', link))).toBe(
          '../lib/tmt-squad/current/tmt-squad'
        );
      expect(parseWholeStdout(await cli(install))).toMatchObject({ changed: false });

      // A foreign same-named command elsewhere on PATH is reported, never run.
      const foreign = path.join(sandbox.root, 'foreign bin');
      mkdirSync(foreign);
      writeFileSync(path.join(foreign, 'tmt-sq'), '#!/bin/sh\nexit 7\n');
      chmodSync(path.join(foreign, 'tmt-sq'), 0o755);
      // A controlled PATH: never the user's own installed commands.
      sandbox.env.PATH = [path.join(prefix, 'bin'), foreign, '/usr/bin', '/bin'].join(':');
      const listed = parseWholeStdout(await cli(['extension', 'list', '--prefix', prefix]));
      expect(listed).toEqual({
        extensions: [
          {
            name: 'office',
            installed: false,
            version: null,
            channel: null,
            pinned: null,
            commands: ['tmt-office'],
            shadowedBy: [],
          },
          {
            name: 'squad',
            installed: true,
            version: '0.1.0-alpha.1',
            channel: 'alpha',
            pinned: null,
            commands: ['tmt-squad', 'tmt-sq'],
            shadowedBy: [path.join(foreign, 'tmt-sq')],
          },
        ],
      });

      // Root help groups the two names that resolve to one file.
      const help = await runCli(sandbox, ['--help']);
      expect(help.stdout).toMatch(/^ {2}squad \(also: sq\) +\S/m);

      expectError(
        await cli(['extension', 'uninstall', 'squad', '--prefix', prefix]),
        'EXTENSION_CONSENT_REQUIRED'
      );
      const removed = await cli(['extension', 'uninstall', 'squad', '--yes', '--prefix', prefix]);
      expect(parseWholeStdout(removed)).toEqual({
        extension: 'squad',
        installed: false,
        changed: true,
        skillsRemoved: [],
        skillsKept: [],
        kept: ['releases', 'hookConsent'],
      });
      for (const link of ['tmt-squad', 'tmt-sq'])
        expect(existsSync(path.join(prefix, 'bin', link))).toBe(false);
      expect(existsSync(path.join(prefix, 'lib/tmt-squad/releases'))).toBe(true);
      expectError(
        await cli(['extension', 'upgrade', 'squad', '--yes', '--prefix', prefix]),
        'EXTENSION_NOT_INSTALLED'
      );
    });
  }, 60_000);

  it('offers bundled skills, refreshes them by name on update, and uninstall removes every owned skill', async () => {
    await withSandbox(async (sandbox) => {
      const prefix = path.join(sandbox.root, 'extension prefix');
      const cli = (args: string[]) =>
        runCli(sandbox, [...args, '--json'], { deadlineMs: INSTALL_PROCESS_BUDGET_MS });
      const install = (artifact: { archive: string; manifest: string }, extra: string[] = []) =>
        cli([
          'extension',
          'install',
          'squad',
          '--yes',
          '--archive',
          artifact.archive,
          '--manifest',
          artifact.manifest,
          '--prefix',
          prefix,
          '--channel',
          'alpha',
          ...extra,
        ]);
      const squad = (version: string, skills: Record<string, string>) =>
        createArtifact(sandbox, version, new Uint8Array(), 'squad', undefined, skills);
      const first = await squad('0.1.0-alpha.1', {
        'tmt-squad/SKILL.md': 'lead v1\n',
        'tmt-squad/references/usage.md': 'usage\n',
        'tmt-squad-retired/SKILL.md': 'retired\n',
      });

      // --yes installs the extension but never publishes skills by itself.
      const offered = parseWholeStdout(await install(first)) as Record<string, unknown>;
      expect(offered.skills).toEqual({
        available: ['tmt-squad', 'tmt-squad-retired'],
        published: [],
        removed: [],
      });
      const human = await runCli(
        sandbox,
        [
          'extension',
          'install',
          'squad',
          '--yes',
          '--archive',
          first.archive,
          '--manifest',
          first.manifest,
          '--prefix',
          prefix,
        ],
        { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
      );
      expect(human.stdout).toContain(
        '2 agent skills available (tmt-squad, tmt-squad-retired); publish with: tmt extension install squad --skills'
      );

      // --skills publishes the verified tree into every provider root.
      const accepted = parseWholeStdout(await install(first, ['--skills'])) as {
        skills: { published: Array<{ name: string; target: string }> };
      };
      const targets = accepted.skills.published.map((item) => item.target);
      expect(targets.length).toBeGreaterThan(0);
      // The provider roots this sandbox publishes into, from the report itself.
      const roots = [...new Set(targets.map((target) => path.dirname(target)))];
      const published = (name: string) =>
        roots
          .map((root) => path.join(root, name))
          .filter((target) => existsSync(target) && lstatSync(target).isSymbolicLink());
      for (const item of accepted.skills.published) {
        expect(item.target.startsWith(sandbox.home)).toBe(true);
        expect(lstatSync(item.target).isSymbolicLink()).toBe(true);
      }
      const lead = published('tmt-squad');
      expect(lead.length).toBeGreaterThan(0);
      for (const target of lead) {
        expect(readFileSync(path.join(target, 'SKILL.md'), 'utf8')).toBe('lead v1\n');
        expect(readFileSync(path.join(target, 'references/usage.md'), 'utf8')).toBe('usage\n');
      }

      // A playbook the same owner holds, outside the release tree.
      const playbook = await runCli(sandbox, ['api'], {
        stdin: JSON.stringify({
          version: 1,
          operation: 'skills.install',
          input: {
            owner: 'squad',
            consent: true,
            skills: [{ name: 'squad-playbook', files: [{ path: 'SKILL.md', content: 'play' }] }],
          },
        }),
      });
      expect(playbook.status, playbook.stdout).toBe(0);

      // An update refreshes held tree skills, removes the one it dropped by
      // name, and leaves the playbook alone.
      const second = await squad('0.2.0-alpha.1', { 'tmt-squad/SKILL.md': 'lead v2\n' });
      const updated = parseWholeStdout(await install(second)) as {
        version: string;
        skills: { available: string[]; removed: string[] };
      };
      expect(updated.version).toBe('0.2.0-alpha.1');
      expect(updated.skills.available).toEqual(['tmt-squad']);
      expect(updated.skills.removed.length).toBeGreaterThan(0);
      for (const target of published('tmt-squad'))
        expect(readFileSync(path.join(target, 'SKILL.md'), 'utf8')).toBe('lead v2\n');
      expect(published('tmt-squad-retired')).toEqual([]);
      const plays = published('squad-playbook');
      expect(plays.length).toBeGreaterThan(0);

      // A folder nobody manages is never touched.
      const foreign = path.join(path.dirname(plays[0]!), 'someone-elses-skill');
      mkdirSync(foreign);
      writeFileSync(path.join(foreign, 'SKILL.md'), 'mine');

      // Uninstall removes every skill the owner holds, playbooks included.
      const removed = parseWholeStdout(
        await cli(['extension', 'uninstall', 'squad', '--yes', '--prefix', prefix])
      ) as { skillsRemoved: string[]; skillsKept: string[]; kept: string[] };
      expect(removed.skillsRemoved.length).toBe(lead.length + plays.length);
      expect(removed.skillsKept).toEqual([]);
      expect(removed.kept).toEqual(['releases', 'hookConsent']);
      expect(published('tmt-squad')).toEqual([]);
      expect(published('squad-playbook')).toEqual([]);
      expect(readFileSync(path.join(foreign, 'SKILL.md'), 'utf8')).toBe('mine');
      expect(existsSync(path.join(prefix, 'lib/tmt-squad/releases'))).toBe(true);
    });
  }, 90_000);

  it('keeps the extension installed when its skills cannot be published', async () => {
    await withSandbox(async (sandbox) => {
      const prefix = path.join(sandbox.root, 'extension prefix');
      const artifact = await createArtifact(
        sandbox,
        '0.1.0-alpha.1',
        new Uint8Array(),
        'squad',
        undefined,
        { 'tmt-squad/SKILL.md': 'lead\n' }
      );
      // An unmanaged folder already holds the name in the Claude root.
      const conflict = path.join(sandbox.home, '.claude/skills/tmt-squad');
      mkdirSync(conflict, { recursive: true });
      writeFileSync(path.join(conflict, 'SKILL.md'), 'hand-written');
      const result = await runCli(
        sandbox,
        [
          'extension',
          'install',
          'squad',
          '--yes',
          '--skills',
          '--archive',
          artifact.archive,
          '--manifest',
          artifact.manifest,
          '--prefix',
          prefix,
          '--channel',
          'alpha',
          '--json',
        ],
        { deadlineMs: INSTALL_PROCESS_BUDGET_MS }
      );
      expectError(result, 'EXTENSION_SKILLS_FAILED');
      expect(readFileSync(path.join(conflict, 'SKILL.md'), 'utf8')).toBe('hand-written');
      expect(readlinkSync(path.join(prefix, 'bin/tmt-squad'))).toBe(
        '../lib/tmt-squad/current/tmt-squad'
      );
    });
  }, 60_000);
});
