export const HANDOFF_PROBE: string[];
export function hasInstallerHandoff(sourceRoot: URL): boolean;
interface Options {
  cwd: string;
  env: NodeJS.ProcessEnv;
}
type Execute = (command: string, args: string[], options: Options) => string;
export function installerProtocol(
  executable: string,
  options: Options,
  execute?: Execute
): 'handoff' | 'legacy';
export function requireInstallerProtocol(
  executable: string,
  options: Options,
  execute?: Execute
): void;
export function requireLegacyInventoryError(result: {
  error?: { code: string; message: string };
}): void;
interface Artifact {
  version: string;
  target: string;
  name: string;
  companions?: string[];
}
export function proveSourceBootstrap(input: {
  root: string;
  source: string;
  current: Artifact;
  previous: Artifact;
  bootstrap: string;
  archive: string;
  manifest: string;
  previousArchive: string;
  previousManifest: string;
  expectedMigrations: number;
  report?: (message: string) => void;
}): void;
