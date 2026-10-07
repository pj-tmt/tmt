import type { VersionSnapshot } from './release-version-injection.mjs';
export interface ApplicationSchema {
  schema_version: 1;
  product: 'cli';
  source_sha: string;
  databases: { domain: 'tmt-core-db'; version: number }[];
  source_files: { path: string; sha256: string }[];
}
export interface SchemaExport {
  record: ApplicationSchema;
  output_sha256: string;
  binary_sha256: string;
}
export interface TargetSchema extends SchemaExport {
  schema_version: 1;
  target: string;
  source_snapshot_sha256: string;
  archive_sha256: string;
}
export interface ExportInput {
  executable: string;
  target: string;
  snapshot: VersionSnapshot;
  root: string;
}
export type ExecuteSchema = (
  executable: string,
  args: string[],
  options: {
    cwd: string;
    env: Record<string, string>;
    timeoutMs: number;
  }
) => string;
export const CLI_SCHEMA_TARGETS: string[];
export function parseCompiledSchema(text: string): ApplicationSchema;
export function assertCompiledSource(
  record: ApplicationSchema,
  snapshot: VersionSnapshot,
  root: string
): void;
export function exportCompiledSchema(input: ExportInput, execute?: ExecuteSchema): SchemaExport;
export function captureSchema(
  input: ExportInput & { archive: string },
  execute?: ExecuteSchema
): TargetSchema;
export function readSchemaEvidence(file: string): TargetSchema;
export function attachApplicationSchema(input: {
  manifest: Record<string, unknown>;
  evidence: TargetSchema[];
  snapshot: VersionSnapshot;
  root: string;
  directory: string;
}): Record<string, unknown>;
export function verifyApplicationSchema(
  input: ExportInput & {
    manifestBytes: Buffer;
    archive: string;
    evidence: TargetSchema;
  },
  execute?: ExecuteSchema
): SchemaExport & { manifest_sha256: string };
