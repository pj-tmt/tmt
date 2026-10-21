import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';

interface Screen {
  readonly identityId: string;
  readonly host: string;
  readonly format: string;
  readonly text: string;
}

/** `pane.screen` through the public API, as an extension would call it. */
function installScreenReader(fixture: E2EFixture) {
  const reader = path.join(fixture.wrapperDir, 'tmt-screen');
  writeExecutable(reader, '#!/bin/sh\nexec "$TMT_EXECUTABLE" api < "$1"\n', 0o700);
  let sequence = 0;
  return (input: unknown) => {
    sequence += 1;
    const file = path.join(fixture.root, `screen-${sequence}.json`);
    fs.writeFileSync(file, JSON.stringify({ version: 1, operation: 'pane.screen', input }));
    return fixture.runCli<Screen & { error?: { code: string } }>(['screen', file]);
  };
}

describe('pane screen on a live host', { concurrent: false }, () => {
  it('reads a bound pane with its colors, and names a missing identity or pane', async () => {
    await withE2EFixture(async (fixture) => {
      const read = installScreenReader(fixture);
      const shell = fixture.createShellPane('screen-shell').pane;
      const bound = expectJsonResult(
        await fixture.runJsonCli<{ id: string }>(['name', 'ScreenPeer'], { pane: shell })
      );

      fixture.tmux(['send-keys', '-t', shell, '-l', "printf '\\033[31mred\\033[0m shown\\n'"]);
      fixture.tmux(['send-keys', '-t', shell, 'Enter']);
      const deadline = Date.now() + 15_000;
      let screen: Screen | undefined;
      while (!screen) {
        const result = await read({ identityId: bound.id });
        expect(result.code, result.stderr).toBe(0);
        if (result.json!.text.includes('\u001b[31mred')) screen = result.json;
        else if (Date.now() > deadline) throw new Error(`No colored output: ${result.stdout}`);
        else await new Promise((resolve) => setTimeout(resolve, 100));
      }
      expect(screen).toMatchObject({ identityId: bound.id, host: 'tmux', format: 'ansi' });
      expect(screen.text).toContain('shown');

      const errorOf = async (input: unknown) => {
        const result = await read(input);
        expect(result.code).toBe(1);
        return result.json?.error?.code;
      };
      expect(await errorOf({ identityId: '22222222-2222-4222-8222-222222222222' })).toBe(
        'NAME_NOT_FOUND'
      );
      const unbound = expectJsonResult(
        await fixture.runJsonCli<{ identity: { id: string } }>(['identity', 'create', 'NoPane'])
      );
      expect(await errorOf({ identityId: unbound.identity.id })).toBe('PANE_NOT_FOUND');
      expect(await errorOf({ identityId: bound.id, lines: 5 })).toBe('API_INPUT_INVALID');

      fixture.tmux(['kill-pane', '-t', shell]);
      expect(await errorOf({ identityId: bound.id })).toBe('PANE_NOT_FOUND');
    });
  });
});
