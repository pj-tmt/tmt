import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture } from './harness.js';

function quote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

describe('interactive shell completion', { concurrent: false }, () => {
  for (const shell of ['bash', 'zsh', 'fish']) {
    it(`${shell} completes identities and delegates command arguments on real Tab input`, async () => {
      await withE2EFixture(async (fixture) => {
        const executable = [fixture.executables.cli.executable, ...fixture.executables.cli.args]
          .map(quote)
          .join(' ');
        writeExecutable(
          path.join(fixture.wrapperDir, 'tmt'),
          `#!/bin/sh\nexec ${executable} "$@"\n`,
          0o700
        );
        writeExecutable(
          path.join(fixture.wrapperDir, 'tmt-fixture-command'),
          '#!/bin/sh\nexit 98\n',
          0o700
        );
        writeExecutable(
          path.join(fixture.wrapperDir, 'tmt-vault'),
          '#!/bin/sh\n[ "$1" = __complete ] && [ "$2" = -- ] || exit 99\ncase "$3" in --cho*) printf "%s\\n" --choice ;; lit*) printf "%s\\n" "literal value" ;; esac\n',
          0o700
        );
        const created = await fixture.runCli(['identity', 'create', 'Alice Example', '--json'], {
          withoutTmux: true,
        });
        expect(created.code, created.stderr).toBe(0);
        const generated = await fixture.runCli(['__completion-script', shell], {
          withoutTmux: true,
        });
        expect(generated.code, generated.stderr).toBe(0);
        const completionFile = path.join(fixture.root, `completion.${shell}`);
        const setupFile = path.join(fixture.root, `setup.${shell}`);
        const ready = path.join(fixture.root, `ready-${shell}`);
        const capture = path.join(fixture.root, `capture-${shell}`);
        const pendingCapture = `${capture}.pending`;
        // Publish the complete buffer, not the transient empty file after redirection.
        const publishCapture = `mv ${quote(pendingCapture)} ${quote(capture)}`;
        const forbidden = path.join(fixture.root, `forbidden-${shell}`);
        writeExecutable(completionFile, generated.stdout, 0o644);
        const common = `source ${quote(completionFile)}\n`;
        const captureAction = `printf '%s' "$READLINE_LINE" > ${quote(pendingCapture)} && ${publishCapture}`;
        const setup =
          shell === 'bash'
            ? `${common}
_fixture_provider() { COMPREPLY=(--provider-choice); }
complete -F _fixture_provider fake claude
bind -x ${quote(`"\\C-x":${captureAction}`)}
export TMT_E2E_FORBID_TMUX=1 TMT_E2E_FORBIDDEN_TMUX_LOG=${quote(forbidden)}
printf ready > ${quote(ready)}
`
            : shell === 'zsh'
              ? `autoload -Uz compinit
compinit -D
${common}
_fixture_provider() { compadd -- --provider-choice; }
compdef _fixture_provider fake claude
_fixture_capture() { print -rn -- "$BUFFER" > ${quote(pendingCapture)} && ${publishCapture}; }
zle -N _fixture_capture
bindkey '^X' _fixture_capture
export TMT_E2E_FORBID_TMUX=1 TMT_E2E_FORBIDDEN_TMUX_LOG=${quote(forbidden)}
printf ready > ${quote(ready)}
`
              : `${common}
complete -c fake -f -a --provider-choice
complete -c claude -f -a --provider-choice
bind \\cx ${quote(`commandline > ${quote(pendingCapture)}; and ${publishCapture}`)}
set -gx TMT_E2E_FORBID_TMUX 1
set -gx TMT_E2E_FORBIDDEN_TMUX_LOG ${quote(forbidden)}
printf ready > ${quote(ready)}
`;
        writeExecutable(setupFile, setup, 0o644);
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
          ['tmt run clau', 'tmt run claude'],
          ['tmt run claude --prov', 'tmt run claude --provider-choice'],
          ['tmt run Al', 'tmt run Alice\\ Example'],
          ['tmt this Al', 'tmt this Alice\\ Example'],
          ['tmt talk Bob message --identity Al', 'tmt talk Bob message --identity Alice\\ Example'],
          ['tmt talk Bob message --identity=Al', 'tmt talk Bob message --identity=Alice\\ Example'],
          ['tmt run Nobody tmt-fixture-c', 'tmt run Nobody tmt-fixture-command'],
          ['tmt run Nobody fake --prov', 'tmt run Nobody fake --provider-choice'],
          ['tmt vaul', 'tmt vault'],
          ['tmt help vaul', 'tmt help vault'],
          ['tmt vault --cho', 'tmt vault --choice'],
          ['tmt vault lit', 'tmt vault literal\\ value'],
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
