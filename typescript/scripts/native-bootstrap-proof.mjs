import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { runPackedCommand } from './packed-command.mjs';

/** Verification-only curl replacement; real shell utilities and native installation remain unchanged. */
export function bootstrapTools(root) {
  const tools = path.join(root, 'tools');
  fs.mkdirSync(tools);
  for (const utility of ['uname', 'mktemp', 'rm', 'wc', 'tar', 'gzip', 'chmod', 'cp', 'sh']) {
    const executable = runPackedCommand('/bin/sh', ['-c', 'command -v "$1"', 'resolve', utility], {
      cwd: root,
      env: process.env,
    }).trim();
    assert(path.isAbsolute(executable), `Expected a real system ${utility}`);
    fs.symlinkSync(executable, path.join(tools, utility));
  }
  const hashTool = process.platform === 'darwin' ? 'shasum' : 'sha256sum';
  const hashExecutable = runPackedCommand(
    '/bin/sh',
    ['-c', 'command -v "$1"', 'resolve', hashTool],
    {
      cwd: root,
      env: process.env,
    }
  ).trim();
  fs.symlinkSync(hashExecutable, path.join(tools, hashTool));
  // Test-only acquisition replacement. The production installer has no endpoint
  // override. All remaining shell/native work uses real tools and real archives.
  fs.writeFileSync(
    path.join(tools, 'curl'),
    `#!/bin/sh
set -eu
destination=
while [ "$#" -gt 1 ]; do
  case "$1" in --output) destination=$2; shift 2 ;; *) shift ;; esac
done
case "$1" in
  "https://github.com/pj-tmt/tmt/releases/download/v$TMT_FIXTURE_VERSION/dist-manifest.json") source=$TMT_FIXTURE_MANIFEST ;;
  "https://github.com/pj-tmt/tmt/releases/download/v$TMT_FIXTURE_VERSION/$TMT_FIXTURE_NAME") source=$TMT_FIXTURE_ARCHIVE ;;
  *) exit 91 ;;
esac
exec cp "$source" "$destination"
`,
    { mode: 0o755 }
  );
  return tools;
}
