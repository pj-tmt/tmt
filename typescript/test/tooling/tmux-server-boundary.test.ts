import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';

// A server-ending tmux verb run against the wrong server ends every agent pane (#4189). Test
// code, scripts and agent skills therefore name `kill-server` or `kill-session` only in the
// files below, each of which proves its socket private first. Anything else goes through
// `privateTmux`, `scripts/private-tmux.sh` or the e2e fixture's `tmux`.

const repository = fileURLToPath(new URL('../../../', import.meta.url));

/** A verb on its own, not a control-file suffix such as `<log>.kill-server`. */
const VERB = /(?<![.\w-])kill-(?:server|session)\b/;
const EXTENSIONS = new Set(['.ts', '.mts', '.mjs', '.js', '.rs', '.sh', '.py']);
const SKIPPED = new Set([
  'node_modules',
  'target',
  '.git',
  '.pnpm-store',
  'dist',
  'site',
  'design',
]);

const GUARD = 'the private-tmux guard itself';
const FIXTURE = "the e2e fixture's tmux wrapper (unsets TMUX, refuses a foreign socket)";
const COLAB = 'the Colab acceptance harness (private -L name); owned by the Colab squad';

/** Every file allowed to name the verbs, with the reason; each must still do so. */
const ALLOWED: Record<string, string> = {
  'scripts/private-tmux.sh': GUARD,
  'typescript/test/support/private-tmux.ts': GUARD,
  'typescript/test/tooling/private-tmux.test.ts': GUARD,
  'typescript/test/tooling/private-tmux-script.test.ts': GUARD,
  'typescript/test/tooling/tmux-server-boundary.test.ts': GUARD,
  '.agents/skills/tmt-dev/references/focused-checks.md': GUARD,
  '.agents/skills/tmt-e2e/references/test-boundaries.md': GUARD,
  'rust/crates/tmt-adapters/src/tmux/server_id_tests.rs':
    'PrivateServer, which proves its socket private before every command',
  'typescript/test/e2e/harness/fixture.ts': FIXTURE,
  'typescript/test/e2e/multi-server.e2e.test.ts': FIXTURE,
  'typescript/test/e2e/workspace-snapshot.e2e.test.ts': FIXTURE,
  'extensions/tmt-colab/typescript/app/acceptance/harness.spec.ts': COLAB,
  'extensions/tmt-colab/typescript/app/acceptance/harness/world.ts': COLAB,
};

function files(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    if (SKIPPED.has(entry.name)) return [];
    const target = path.join(directory, entry.name);
    if (entry.isDirectory()) return files(target);
    const agentSkill = target.includes(`${path.sep}.agents${path.sep}`) && target.endsWith('.md');
    return EXTENSIONS.has(path.extname(entry.name)) || agentSkill ? [target] : [];
  });
}

function mentions(): Map<string, number[]> {
  const found = new Map<string, number[]>();
  for (const file of files(repository)) {
    const lines = readFileSync(file, 'utf8')
      .split('\n')
      .flatMap((line, index) => (VERB.test(line) ? [index + 1] : []));
    if (lines.length > 0)
      found.set(path.relative(repository, file).split(path.sep).join('/'), lines);
  }
  return found;
}

describe('server-ending tmux verbs', () => {
  it('are named only by files that prove their socket private', () => {
    const offenders = [...mentions()]
      .filter(([file]) => !(file in ALLOWED))
      .map(([file, lines]) => `${file}:${lines.join(',')}`);
    expect(
      offenders,
      'Use privateTmux (typescript/test/support), scripts/private-tmux.sh or the e2e fixture tmux() instead of naming kill-server or kill-session'
    ).toEqual([]);
  });

  it('are still named by every allowed file, so the list only shrinks', () => {
    const found = mentions();
    expect(Object.keys(ALLOWED).filter((file) => !found.has(file))).toEqual([]);
  });

  it('recognise a verb and ignore a control-file suffix or another kill command', () => {
    for (const line of [
      'tmux kill-server',
      "run(['kill-server'])",
      'b.tmux(["kill-session", "-t", x])',
      'self.command(&["kill-server"])',
    ])
      expect(VERB.test(line), line).toBe(true);
    for (const line of [
      '`${log}.kill-server`',
      "fixture.tmux(['kill-pane', '-t', pane])",
      'kill-window',
      'a-kill-server-b',
    ])
      expect(VERB.test(line), line).toBe(false);
  });
});
