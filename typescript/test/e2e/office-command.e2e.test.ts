import { mkdirSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import path from 'node:path';
import { expect, it } from 'vitest';
import { resolveCliExecutables } from '../support/cli-executable.mjs';
import { createArtifact } from '../support/native-artifact.js';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type CliRunOptions, type E2EFixture } from './harness.js';

async function office(fixture: E2EFixture) {
  const companion = '/workspace/rust/target/debug/tmt-office';
  const prefix = path.join(fixture.root, 'isolated office');
  const home = path.join(fixture.root, 'office-home');
  mkdirSync(home);
  const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
  // Only installation environment differs: optional skills must stay below
  // the fixture root, not survive under the shared container home. The real
  // dispatcher supplies TMT_EXECUTABLE; exec preserves caller process evidence.
  const environment = `HOME=${quote(home)} CODEX_HOME=${quote(path.join(home, '.codex'))} XDG_CONFIG_HOME=${quote(path.join(home, '.config'))}`;
  for (const [name, command] of [
    ['officefacade', '"$TMT_EXECUTABLE" office'],
    ['officedirect', quote(companion)],
  ]) {
    writeExecutable(
      path.join(fixture.wrapperDir, `tmt-${name}`),
      `#!/bin/sh\nexec env ${environment} ${command} "$@"\n`,
      0o755
    );
  }
  const run = (entry: string, args: string[], options: CliRunOptions = {}) =>
    fixture.runCli([entry, '--prefix', prefix, ...args, '--json'], options);
  const artifact = await createArtifact(
    fixture,
    '0.1.0-alpha.4',
    new Uint8Array(),
    'office',
    companion
  );
  expectJsonResult(
    await fixture.runJsonCli([
      '__native-install',
      '--product',
      'office',
      '--channel',
      'alpha',
      '--prefix',
      prefix,
      '--archive',
      artifact.archive,
      '--manifest',
      artifact.manifest,
    ])
  );
  return async (args: string[], options: CliRunOptions = {}) => {
    const facade = await run('officefacade', args, options);
    const direct = await run('officedirect', args, options);
    expect(direct.code, direct.stdout + direct.stderr).toBe(facade.code);
    expect(direct.json).toEqual(facade.json);
    expect(direct.stderr).toBe(facade.stderr);
    return facade;
  };
}

it(
  'preserves identity and historical room behavior through both Office core-access implementations',
  { timeout: 120_000 },
  async () => {
    await withE2EFixture(async (fixture) => {
      const compare = await office(fixture);
      const alice = expectJsonResult(
        await fixture.runJsonCli<{ id: string }>(['name', 'Alice', '-s'])
      );
      for (const args of [[], ['--identity', 'Alice'], ['--identity', alice.id]]) {
        const profile = await compare(['profile', 'show', '--local', ...args]);
        expect(profile.code).toBe(0);
      }
      const unbound = await compare(['profile', 'show', '--local'], { outsideTmux: true });
      expect(unbound).toMatchObject({ code: 1, json: { error: { code: 'IDENTITY_REQUIRED' } } });
      expect(await compare(['profile', 'show', '--local', '--identity', '   '])).toMatchObject({
        code: 3,
        json: { error: { code: 'NAME_NOT_FOUND' } },
      });
      const room = expectJsonResult(
        await fixture.runJsonCli<{ room: { id: string } }>(['room', 'create', 'Design'])
      ).room;
      expect((await compare(['board', 'list', '--room', room.id])).code).toBe(0);
      expectJsonResult(await fixture.runJsonCli(['room', 'retire', room.id]));
      expect((await compare(['board', 'list', '--room', room.id])).code).toBe(0);
      expectJsonResult(await fixture.runJsonCli(['room', 'create', 'Design']));
      expectJsonResult(await fixture.runJsonCli(['room', 'create', 'Design']));
      expect(await compare(['board', 'list', '--room', 'Design'])).toMatchObject({
        code: 1,
        json: { error: { code: 'ROOM_AMBIGUOUS' } },
      });
      expect(await compare(['board', 'list', '--room', 'Missing'])).toMatchObject({
        code: 3,
        json: { error: { code: 'ROOM_NOT_FOUND' } },
      });
      expectJsonResult(await fixture.runJsonCli(['rm', 'Alice', '--force']));
      expect(await compare(['profile', 'show', '--local', '--identity', alice.id])).toMatchObject({
        code: 3,
        json: { error: { code: 'NAME_NOT_FOUND' } },
      });
    });
  }
);

it(
  'preserves shared-host rejection without losing explicit Office identity selection',
  { timeout: 120_000 },
  async () => {
    const selected = resolveCliExecutables();
    await withE2EFixture(
      async (fixture) => {
        const compare = await office(fixture);
        expectJsonResult(await fixture.runJsonCli(['add', fixture.pane, 'Alice', '-s']));
        const rejected = await compare(['profile', 'show', '--local']);
        expect(rejected).toMatchObject({
          code: 1,
          json: { error: { code: 'CALLER_IDENTITY_AMBIGUOUS' } },
        });
        expect((await compare(['profile', 'show', '--local', '--identity', 'Alice'])).code).toBe(0);
      },
      {
        executableEnv: {
          ...process.env,
          TMT_TEST_CLI: JSON.stringify({
            executable: '/opt/tmt-tests/codex',
            args: ['app-server', selected.cli.executable, ...selected.cli.args],
          }),
          TMT_TEST_PEER_CLI: JSON.stringify(selected.peer),
        },
      }
    );
  }
);
