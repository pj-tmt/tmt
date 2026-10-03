import type { ComponentMap } from './ci-scope.mjs';
export interface InjectionMetadata {
  workspace_members: string[];
  packages: { id: string; name: string; version: string; manifest_path: string }[];
}
export interface VersionSnapshot {
  schema: number;
  cut: string;
  product: string;
  tag: string;
  version: string;
  oldVersion: string;
  manifest: string;
  section: string;
  source: string;
  packages: string[];
  lock: string;
  hashes: Record<string, string>;
}
export function captureVersionState(input: {
  root: string;
  files: string[];
  metadata: InjectionMetadata;
  product: string;
  tag: string;
  cut: string;
  map: ComponentMap;
}): VersionSnapshot;
export function injectVersion(root: string, snapshot: VersionSnapshot): void;
export function verifyVersionState(
  root: string,
  snapshot: VersionSnapshot,
  metadata: InjectionMetadata
): {
  product: string;
  tag: string;
  cut: string;
  changed: string[];
  packages: string[];
};
export function verifyDistVersions(
  snapshot: VersionSnapshot,
  plan: { announcement_tag: string; releases: { app_name: string; app_version: string }[] },
  build: { announcement_tag: string; releases: { app_name: string; app_version: string }[] },
  reportedVersion: string
): void;
