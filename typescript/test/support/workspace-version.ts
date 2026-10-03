import { readWorkspace, type Workspace } from '../../scripts/cargo-workspace.mjs';

let crates: Workspace['crates'] | undefined;

/** A crate's Cargo-resolved version, with workspace metadata read once per test suite. */
export function workspaceVersion(crateName: string): string {
  crates ??= readWorkspace().crates;
  const crate = crates.find(({ name }) => name === crateName);
  if (!crate) throw new Error(`Cargo workspace has no ${crateName} crate.`);
  return crate.version;
}
