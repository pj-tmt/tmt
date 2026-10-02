import fs from 'node:fs';
import path from 'node:path';
import type { E2EFixture } from './harness.js';
import { TMUX_COMMAND_INSPECTION } from './tmux-command-inspection.js';

export interface TmuxTrace {
  readonly path: string;
  readonly clear: () => void;
  readonly invocations: () => string[];
  readonly commands: () => string[];
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

/**
 * Add a narrow invocation trace around the fixture's existing tmux wrapper.
 * The wrapper remains responsible for the private socket and all existing
 * failure injection; this layer only records subprocess fan-out from a CLI.
 */
export function installTmuxTrace(fixture: E2EFixture): TmuxTrace {
  const wrapperPath = path.join(fixture.wrapperDir, 'tmux');
  const delegatedWrapperPath = path.join(fixture.wrapperDir, 'tmux-inner');
  const tracePath = path.join(fixture.root, 'tmux-command-trace.log');

  if (fs.existsSync(delegatedWrapperPath)) {
    throw new Error('Tmux trace is already installed for this fixture. Reuse and clear it.');
  }
  fs.renameSync(wrapperPath, delegatedWrapperPath);
  fs.writeFileSync(
    wrapperPath,
    `#!/bin/sh
trace_newline='
'
trace_args="${'$'}*"
# Normalize only the trace copy; delegated tmux receives the original "${'$'}@".
case "${'$'}trace_args" in
  *"${'$'}trace_newline"*) trace_args=$(printf '%s' "${'$'}trace_args" | tr '\n' ' ') ;;
esac
${TMUX_COMMAND_INSPECTION}
printf '%s\\t%s\\n' "${'$'}tmux_command" "${'$'}trace_args" >> ${shellQuote(tracePath)}
exec ${shellQuote(delegatedWrapperPath)} "${'$'}@"
`,
    { mode: 0o755 }
  );

  fs.writeFileSync(tracePath, '');

  return {
    path: tracePath,
    clear: () => fs.writeFileSync(tracePath, ''),
    invocations: () => fs.readFileSync(tracePath, 'utf8').split('\n').filter(Boolean),
    commands: () =>
      fs
        .readFileSync(tracePath, 'utf8')
        .split('\n')
        .filter(Boolean)
        .map((invocation) => invocation.slice(0, invocation.indexOf('\t'))),
  };
}
