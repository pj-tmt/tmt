import { writeExecutable } from '../support/executable-fixture.mjs';
import Database from 'better-sqlite3';
import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { durableState } from './identity-state-oracle.js';
import { waitForFileContent } from './wait-for-file.js';

function quote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

/** `changes.cursor` through the public API, as an extension would call it. */
function installCursorReader(fixture: E2EFixture): () => Promise<number> {
  const input = path.join(fixture.root, 'cursor.json');
  fs.writeFileSync(input, JSON.stringify({ version: 1, operation: 'changes.cursor', input: {} }));
  const reader = path.join(fixture.wrapperDir, 'tmt-cursor');
  writeExecutable(reader, `#!/bin/sh\nexec "$TMT_EXECUTABLE" api < '${input}'\n`, 0o700);
  return async () => expectJsonResult(await fixture.runCli<{ cursor: number }>(['cursor'])).cursor;
}

describe('change cursor on a live host', { concurrent: false }, () => {
  it('stays put across repeated reads of bound, running and remembered state, and moves on real transitions', async () => {
    await withE2EFixture(async (fixture) => {
      const cursor = installCursorReader(fixture);
      // A bound pane that the reads below reconcile every time.
      expectJsonResult(await fixture.runJsonCli(['name', 'Ada']));

      // A second pane with a running foreground session.
      const shell = fixture.createShellPane('cursor-runner').pane;
      const ready = path.join(fixture.root, 'runner-ready.json');
      const fake = path.join(fixture.wrapperDir, 'cursor-harness');
      writeExecutable(
        fake,
        `#!${process.execPath}\nrequire('node:fs').writeFileSync(${JSON.stringify(ready)}, JSON.stringify({ child: process.pid }));\nsetInterval(() => {}, 1000);\n`,
        0o700
      );
      const command = [fixture.executables.cli.executable, ...fixture.executables.cli.args]
        .concat(['run', '-s', 'Bob', fake])
        .map(quote)
        .join(' ');
      fixture.tmux(['send-keys', '-t', shell, '-l', command]);
      fixture.tmux(['send-keys', '-t', shell, 'Enter']);
      const runner = JSON.parse(
        await waitForFileContent(ready, { description: 'runner start' })
      ) as { child: number };
      const runtime = () =>
        durableState(fixture).bindings.find((row) => row.pane_id === shell)?.runtime_state;
      await fixture.waitFor(() => runtime() === 'running', 5000, 'running admission');

      // A remembered session for the running identity.
      const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'));
      try {
        db.prepare(
          `INSERT OR REPLACE INTO identity_session_preferences
             (identity_id, preferred_harness, remembered_harness, runtime_mode, provider_session_id)
           SELECT id, 'claude', 'claude', 'default', '12345678-1234-4234-8234-123456789abc'
           FROM identities WHERE name = 'Bob'`
        ).run();
      } finally {
        db.close();
      }

      const settled = await cursor();
      for (let round = 0; round < 3; round += 1) {
        for (const args of [
          ['ls'],
          ['ls', '--here'],
          ['inbox', '--identity', 'Ada'],
          ['inbox', '--identity', 'Bob'],
        ]) {
          expectJsonResult(await fixture.runJsonCli(args));
          expect(await cursor(), `${args.join(' ')} (round ${round})`).toBe(settled);
        }
        const checked = await fixture.runCli(['check', 'Bob', '5']);
        expect(checked.code, checked.stderr).toBe(0);
        expect(await cursor(), `check (round ${round})`).toBe(settled);
      }

      // The runtime ends: a real transition.
      process.kill(runner.child, 'SIGTERM');
      await fixture.waitFor(() => runtime() === 'ended', 5000, 'runtime end');
      const ended = await cursor();
      expect(ended).not.toBe(settled);

      // The pane goes away: the next reconcile detaches it, and that moves it too.
      fixture.tmux(['kill-pane', '-t', shell]);
      expectJsonResult(await fixture.runJsonCli(['ls']));
      expect(durableState(fixture).bindings.some((row) => row.pane_id === shell)).toBe(false);
      const detached = await cursor();
      expect(detached).not.toBe(ended);
      expectJsonResult(await fixture.runJsonCli(['ls']));
      expect(await cursor()).toBe(detached);
    });
  });
});
