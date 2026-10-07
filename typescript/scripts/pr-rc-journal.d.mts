import type { RCProducer, RCSnapshot, RCRequest, RCPlan } from './pr-rc-resources.mjs';
export interface RCCheckpointWriter {
  repository: string;
  ownerId: number;
  producer: RCProducer;
  runId: number;
  attempt: number;
}
export interface RCCheckpoint {
  schema: 1;
  writer: RCCheckpointWriter;
  revision: number;
  sourceSha: string;
  predecessor: { id: number; digest: string } | null;
  snapshot: RCSnapshot;
  snapshotDigest: string;
  terminal: { generationKey: string; reference: string }[];
}
export interface RCCheckpointCandidate {
  bytes: Uint8Array;
  digest: string;
  charges: RCPlan['charges'];
}
export interface RCCheckpointRecord extends RCCheckpointCandidate {
  id: number;
}
export interface RCCustody {
  writer: RCCheckpointWriter;
  reference: string;
  concurrencyDomain: string;
  cancelInProgress: boolean;
}
export interface RCCheckpointResponse {
  status: number;
  body: Uint8Array;
  elapsedMs: number;
  nextPage: number | null;
  authenticated: { repository: string; ownerId: number };
}
export interface RCCheckpointEvidence {
  method: string;
  path: string;
  response: RCCheckpointResponse;
}
export interface RCCheckpointRecovery {
  candidate: RCCheckpointCandidate;
  recorded: RCCheckpointRecord[];
  custody: RCCustody;
  phase:
    | 'before-create'
    | 'after-returned-id'
    | 'after-readback'
    | 'before-inactive-status'
    | 'after-returned-status-id'
    | 'after-inactive-status'
    | 'before-delete'
    | 'after-delete'
    | 'complete';
  createdId: number | null;
  statusId: number | null;
  pruningId: number | null;
  evidence: RCCheckpointEvidence[];
}
export interface RCCheckpointPorts {
  /** Original deadline includes queue/custody wait. Must be monotonic milliseconds. */
  now(): number;
  /** External trusted qualification owns exclusivity/authentication; these fields attest to neither. */
  custody: { capture(): Promise<RCCustody> };
  /** No default/live implementation. Must enforce byte/time bounds, authentication and abort. */
  transport(request: {
    method: string;
    path: string;
    body: Uint8Array | null;
    timeoutMs: number;
    maxResponseBytes: number;
    signal: AbortSignal;
  }): Promise<RCCheckpointResponse>;
  /** Durable before return, exact byte preservation; no live storage implementation is supplied. */
  recovery: {
    load(): Promise<RCCheckpointRecovery | null>;
    save(state: RCCheckpointRecovery): Promise<void>;
  };
}
export interface RCCheckpointResult {
  mode: 'unarmed';
  status: 'frozen' | 'refused' | 'readback-confirmed';
  reason?: string;
  successor?: RCCheckpointRecord;
  /** Null means complete recorded-chain accounting could not be established, never zero. */
  charges: RCPlan['charges'];
  /** Original complete records remain available even when chain admission refuses. */
  recorded: RCCheckpointRecord[];
  recovery: RCCheckpointRecovery | null;
  evidence: RCCheckpointEvidence[];
  requests: number;
  transferredBytes: number;
}
export function encodeRCCheckpoint(checkpoint: RCCheckpoint): Uint8Array;
export function decodeRCCheckpoint(bytes: Uint8Array): RCCheckpoint;
export function prepareRCCheckpoint(input: {
  writer: RCCheckpointWriter;
  sourceSha: string;
  previous: RCCheckpointRecord | null;
  snapshot: RCSnapshot;
  request: RCRequest;
  terminal: RCCheckpoint['terminal'];
}): RCCheckpointCandidate;
export function commitRCCheckpoint(input: {
  candidate: RCCheckpointCandidate;
  recorded: RCCheckpointRecord[];
  bootstrap: {
    writer: RCCheckpointWriter;
    admissionReference: string;
    emptySnapshotDigest: string;
  } | null;
  ports: RCCheckpointPorts;
  startedAtMs: number;
}): Promise<RCCheckpointResult>;
