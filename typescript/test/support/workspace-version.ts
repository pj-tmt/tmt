import { readWorkspace } from '../../scripts/release-please-config.mjs';

let cliVersion: string | undefined;

/** The version `tmt --version` prints, resolved by Cargo once per test suite. */
export function workspaceVersion(): string {
  if (cliVersion === undefined) {
    const cli = readWorkspace().crates.find(({ name }) => name === 'tmt-cli');
    if (!cli) throw new Error('Cargo workspace has no tmt-cli crate.');
    cliVersion = cli.version;
  }
  return cliVersion;
}
