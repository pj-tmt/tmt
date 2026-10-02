import { writeExecutable } from './executable-fixture.mjs';
import fs from 'node:fs';
import path from 'node:path';
import type { CliExecutable } from './cli-executable.mjs';

/** A non-Node executable that records argv before execing the real selected CLI. */
export function createCliProbe(
  root: string,
  delegate: CliExecutable
): {
  descriptor: CliExecutable;
  invocations: () => string[][];
} {
  const executable = path.join(root, "CLI probe's executable");
  const log = path.join(root, 'CLI invocations');
  writeExecutable(
    executable,
    `#!/bin/sh
log="$1"
shift
[ "$1" = "prefix with spaces and 'quotes'" ] || exit 91
shift
printf '%s\\0' '__TMT_PROBE_INVOCATION__' "$@" >> "$log"
exec "$@"
`,
    0o755
  );
  return {
    descriptor: {
      executable,
      args: [log, "prefix with spaces and 'quotes'", delegate.executable, ...delegate.args],
    },
    invocations: () =>
      fs.existsSync(log)
        ? fs
            .readFileSync(log, 'utf8')
            .split('__TMT_PROBE_INVOCATION__\0')
            .slice(1)
            .map((entry) => entry.split('\0').slice(0, -1))
        : [],
  };
}
