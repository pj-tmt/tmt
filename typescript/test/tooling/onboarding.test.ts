import { mkdirSync, readFileSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { withSandbox } from '../support/cli-process.js';

const guide = readFileSync(
  new URL('../../../rust/archive/NATIVE-INSTALL.md', import.meta.url),
  'utf8'
);
const setup = guide.split('## One-time PATH setup\n')[1]?.match(/```sh\n([\s\S]*?)\n```/)?.[1];

describe('documented one-time PATH setup', () => {
  it.each(['bash', 'zsh'])(
    'selects the new command without duplicating inherited PATH in %s',
    async (shell) => {
      expect(
        setup,
        'The installation guide must contain its executable PATH example.'
      ).toBeDefined();
      await withSandbox(async (sandbox) => {
        const home = path.join(sandbox.root, 'home with spaces');
        const bin = path.join(home, '.local', 'bin');
        const command = path.join(bin, 'tmt');
        const original = path.join(sandbox.root, 'original-bin');
        mkdirSync(bin, { recursive: true });
        mkdirSync(original);
        // A lookup target only; this fixture never pretends to run the product.
        writeExecutable(command, '#!/bin/sh\nexit 0\n', 0o700);
        for (const [initial, expected] of [
          [original, `${bin}:${original}`],
          [`${bin}:${original}`, `${bin}:${original}`],
          [`${original}:${bin}`, `${original}:${bin}`],
          [`${bin}-other:${original}`, `${bin}:${bin}-other:${original}`],
        ]) {
          const output = execFileSync(
            `/bin/${shell}`,
            ['-c', `${setup}\n${setup}\nprintf '%s\\n' "$PATH"\ncommand -v tmt`],
            {
              cwd: sandbox.cwd,
              env: { HOME: home, PATH: initial },
              encoding: 'utf8',
              timeout: 5000,
              maxBuffer: 64 * 1024,
            }
          );
          expect(output).toBe(`${expected}\n${command}\n`);
        }
      });
    }
  );
});
