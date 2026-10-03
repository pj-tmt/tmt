import { fileURLToPath } from 'node:url';
import { readCargoWorkspace } from '../../scripts/cargo-workspace.mjs';

let crates: ReturnType<typeof readCargoWorkspace>['packages'] | undefined;

/** A crate's Cargo-resolved version, with workspace metadata read once per test suite. */
export function workspaceVersion(crateName: string): string {
  crates ??= readCargoWorkspace(fileURLToPath(new URL('../../../', import.meta.url))).packages;
  const crate = crates.find(({ name }) => name === crateName);
  if (!crate) throw new Error(`Cargo workspace has no ${crateName} crate.`);
  return crate.version;
}
