import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { withE2EFixture } from './harness.js';

function quote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

describe.sequential('interactive shell completion', () => {
  for (const shell of ['bash', 'zsh', 'fish']) {
    it(`${shell} completes identities and delegates command arguments on real Tab input`, async () => {
      await withE2EFixture(async (fixture) => {
        const executable = [fixture.executables.cli.executable, ...fixture.executables.cli.args]
          .map(quote)
          .join(' ');
        writeFileSync(
          path.join(fixture.wrapperDir, 'tmt'),
          `#!/bin/sh\nexec ${executable} "$@"\n`,
          { mode: 0o700 }
        );
        writeFileSync(
          path.join(fixture.wrapperDir, 'tmt-fixture-command'),
          '#!/bin/sh\nexit 98\n',
          { mode: 0o700 }
        );
        const created = await fixture.runCli(['identity', 'create', 'Alice Example', '--json'], {
          withoutTmux: true,
        });
        expect(created.code, created.stderr).toBe(0);
        const generated = await fixture.runCli(['completion', shell], { withoutTmux: true });
        expect(generated.code, generated.stderr).toBe(0);
        const completionFile = path.join(fixture.root, `completion.${shell}`);
        const setupFile = path.join(fixture.root, `setup.${shell}`);
        const ready = path.join(fixture.root, `ready-${shell}`);
        const capture = path.join(fixture.root, `capture-${shell}`);
        const forbidden = path.join(fixture.root, `forbidden-${shell}`);
        writeFileSync(completionFile, generated.stdout);
        const common = `source ${quote(completionFile)}\n`;
        const captureAction = `printf '%s' "$READLINE_LINE" > ${quote(capture)}`;
        const setup =
          shell === 'bash'
            ? `${common}
_fixture_provider() { COMPREPLY=(--provider-choice); }
complete -F _fixture_provider fake
bind -x ${quote(`"\\C-x":${captureAction}`)}
export TMT_E2E_FORBID_TMUX=1 TMT_E2E_FORBIDDEN_TMUX_LOG=${quote(forbidden)}
printf ready > ${quote(ready)}
`
            : shell === 'zsh'
              ? `autoload -Uz compinit
compinit -D
${common}
_fixture_provider() { compadd -- --provider-choice; }
compdef _fixture_provider fake
_fixture_capture() { print -rn -- "$BUFFER" > ${quote(capture)}; }
zle -N _fixture_capture
bindkey '^X' _fixture_capture
export TMT_E2E_FORBID_TMUX=1 TMT_E2E_FORBIDDEN_TMUX_LOG=${quote(forbidden)}
printf ready > ${quote(ready)}
`
              : `${common}
complete -c fake -f -a --provider-choice
bind \\cx ${quote(`commandline > ${quote(capture)}`)}
set -gx TMT_E2E_FORBID_TMUX 1
set -gx TMT_E2E_FORBIDDEN_TMUX_LOG ${quote(forbidden)}
printf ready > ${quote(ready)}
`;
        writeFileSync(setupFile, setup);
        const pane = fixture.createShellPane(`completion-${shell}`).pane;
        if (shell !== 'bash') {
          fixture.tmux([
            'send-keys',
            '-t',
            pane,
            '-l',
            shell === 'zsh' ? 'exec zsh -f' : 'exec fish --no-config',
          ]);
          fixture.tmux(['send-keys', '-t', pane, 'Enter']);
        }
        fixture.tmux(['send-keys', '-t', pane, '-l', `source ${quote(setupFile)}`]);
        fixture.tmux(['send-keys', '-t', pane, 'Enter']);
        await fixture.waitFor(() => existsSync(ready), 5000, `${shell} completion initialization`);
        for (const [input, expected] of [
          ['tmt run Al', 'tmt run Alice\\ Example'],
          ['tmt this Al', 'tmt this Alice\\ Example'],
          ['tmt talk Bob message --identity Al', 'tmt talk Bob message --identity Alice\\ Example'],
          ['tmt talk Bob message --identity=Al', 'tmt talk Bob message --identity=Alice\\ Example'],
          ['tmt run Nobody tmt-fixture-c', 'tmt run Nobody tmt-fixture-command'],
          ['tmt run Nobody fake --prov', 'tmt run Nobody fake --provider-choice'],
          ["tmt run 'Alice Example' fake --prov", "tmt run 'Alice Example' fake --provider-choice"],
          ...(shell === 'bash'
            ? [
                ['tmt run "Al', 'tmt run "Alice Example"'],
                ["tmt run 'Al", "tmt run 'Alice Example'"],
              ]
            : []),
        ]) {
          writeFileSync(capture, 'not captured');
          fixture.tmux(['send-keys', '-t', pane, 'C-u']);
          fixture.tmux(['send-keys', '-t', pane, '-l', input]);
          fixture.tmux(['send-keys', '-t', pane, 'Tab', 'C-x']);
          await fixture.waitFor(
            () => readFileSync(capture, 'utf8') !== 'not captured',
            5000,
            `${shell} line capture`
          );
          const line = readFileSync(capture, 'utf8').trim();
          expect(line, `${shell}: ${fixture.tmux(['capture-pane', '-p', '-t', pane])}`).toBe(
            expected
          );
        }
        expect(existsSync(forbidden)).toBe(false);
      });
    });
  }
});
