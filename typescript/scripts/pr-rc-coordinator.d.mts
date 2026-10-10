import type { ApplicationSchema } from './native-application-schema.mjs';
export const RC_REPOSITORY: 'pj-tmt/tmt';
export const RC_TARGETS: string[];
export const RC_TTL_MS: number;
export interface RCMember {
  name: string;
  sha256: string;
  bytes: number;
}
export interface RCVerificationReport {
  schema_version: 1;
  product: 'cli';
  target: string;
  source_sha: string;
  version: string;
  prepare_run_id: number;
  prepare_run_attempt: number;
  prepare_tooling_sha: string;
  application_schema: ApplicationSchema;
  dist_manifest: RCMember;
  archive: RCMember;
  binary_sha256: string;
  output_sha256: string;
  notices: RCMember;
  source_snapshot_sha256: string;
}
export interface RCIdentity {
  pr: number;
  head: string;
  head12: string;
  runId: number;
  attempt: number;
}
export interface RCObservation {
  pull: unknown;
  timeline: unknown;
  run: unknown;
}
export interface RCEpoch {
  label: 'rc-build';
  label_id: number;
  enabled_event_id: number;
  enabled_at_ms: number;
}
export interface RCCandidate {
  product: 'cli';
  target: string;
  version: string;
  application_schema: ApplicationSchema;
  dist_manifest: RCMember;
  archive: RCMember;
  verification: {
    prepare_run_id: number;
    prepare_run_attempt: number;
    prepare_tooling_sha: string;
    source_sha: string;
    complete: true;
  };
}
export interface RCPayload {
  id: number;
  name: string;
  zip_sha256: string;
  zip_bytes: number;
  members: RCMember[];
}
export interface RCProducer {
  workflow_id: number;
  workflow_path: string;
  workflow_sha256: string;
  tooling_sha: string;
  run_id: number;
  run_attempt: number;
}
export function capturePRRCVerification(input: {
  root: string;
  directory: string;
  target: string;
  snapshot: string;
  runId: number;
  attempt: number;
  toolingSha: string;
}): RCVerificationReport;
export function validatePRRCEligibility(
  observation: RCObservation,
  identity: RCIdentity,
  nowMs: number
): RCEpoch;
export function stagePRRCPayloads(input: {
  directory: string;
  reports: RCVerificationReport[];
  output: string;
  head: string;
  runId: number;
  attempt: number;
  toolingSha: string;
}): RCCandidate[];
export function prRCPayloadName(pr: number, target: string, runId: number, attempt: number): string;
export function prRCCatalogName(pr: number): string;
export function buildPRRCCatalog(
  input: {
    identity: RCIdentity;
    candidates: RCCandidate[];
    producer: RCProducer;
    publishedAtMs: number;
  },
  ports: {
    observe(): RCObservation | Promise<RCObservation>;
    payload(name: string): RCPayload | Promise<RCPayload>;
  }
): Promise<Buffer>;
export interface RCAPI {
  metadata(route: string): unknown;
  inventory(runId: number): unknown[];
  members(artifact: unknown, maximum: number): { name: string; bytes: Buffer }[];
}
export type RCExecute = (
  file: string,
  args: string[],
  options: { input?: Buffer; timeout: number; maxBuffer: number }
) => Buffer;
export function createPRRCAPI(execute?: RCExecute): RCAPI;
export function runPRRCCommand(
  command: string,
  env?: NodeJS.ProcessEnv,
  api?: RCAPI
): Promise<void>;
