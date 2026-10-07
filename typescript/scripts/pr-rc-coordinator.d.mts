import type { RCIdentity, RCProducer, RCSnapshot, RCPlan } from './pr-rc-resources.mjs';
import type {
  RCCheckpointWriter,
  RCCheckpointRecord,
  RCCheckpointPorts,
  RCCheckpointRecovery,
  RCCheckpointResult,
} from './pr-rc-journal.mjs';

/** Local injected source interfaces, never frozen Core wire or a live approval set. */
export interface RCPublicationApproval {
  reference: string;
  producer: RCProducer;
  preparation: {
    workflow_id: number;
    workflow_path: string;
    workflow_sha256: string;
    tooling_sha: string;
    /** Complete externally reviewed workflow/action/verifier/dependency file closure. */
    closure: { path: string; sha256: string }[];
  };
}
export interface RCPublicationResponse {
  status: number;
  body: Uint8Array;
  elapsedMs: number;
  nextPage: number | null;
  reference: string;
  observedAtMs: number;
  authenticated: { repository: string; ownerId: number };
}
export interface RCPublicationLimits {
  timeoutMs: number;
  maximum: number;
  signal: AbortSignal;
}
export interface RCUploadResponse extends RCPublicationResponse {
  /** Actual returned ID, durably retained before interpreting/readback of raw body. */
  id: number | null;
}
export interface RCPublicationPorts {
  checkpoint: Omit<RCCheckpointPorts, 'recovery'> & {
    /** Same checkpoint recovery owner and store; preserves the extended record exactly. */
    recovery: {
      load(): Promise<RCCheckpointRecovery | RCPublicationRecovery | null>;
      save(state: RCCheckpointRecovery | RCPublicationRecovery): Promise<void>;
    };
  };
  /** Must be external immutable owning review, never PR JSON or successful CI. */
  approval: { capture(limits: RCPublicationLimits): Promise<RCPublicationApproval> };
  /** Authenticated bounded exports; no live backend is supplied or qualified here. */
  observe(
    request: RCPublicationLimits & {
      kind: 'eligibility' | 'preparation' | 'artifact' | 'finalization';
      identity: RCIdentity;
      artifactId: number | null;
    }
  ): Promise<RCPublicationResponse>;
  /** Existing upload owner packages exactly these root regular byte members; no PR execution. */
  upload(
    request: RCPublicationLimits & {
      name: string;
      files: { name: string; bytes: Uint8Array }[];
      identity: RCIdentity;
      maxZipBytes: number;
    }
  ): Promise<RCUploadResponse>;
}
export interface RCUploadedResource {
  name: string;
  kind: 'payload' | 'catalog';
  members: { name: string; sha256: string; bytes: number }[];
  phase: 'intent' | 'returned' | 'readback' | 'finalized';
  id: number | null;
  /** Also retain an independently parsed raw-body ID when the receipt disagrees. */
  rawReturnedId: number | null;
  response: RCUploadResponse | null;
  readback: RCPublicationResponse | null;
  finalization: RCPublicationResponse | null;
}
export interface RCPublicationRecovery extends RCCheckpointRecovery {
  /** Recovery evidence alongside the unchanged immutable worst-case checkpoint reservation. */
  publication: {
    generationKey: string;
    reservationId: number;
    reservationDigest: string;
    approvalReference: string;
    phase: 'reserved' | 'uploading' | 'readback-confirmed' | 'frozen';
    observations: { kind: string; artifactId: number | null; response: RCPublicationResponse }[];
    uploads: RCUploadedResource[];
    catalog: unknown;
    unknown: boolean;
  };
}
export interface RCPublicationInput {
  identity: RCIdentity;
  writer: RCCheckpointWriter;
  snapshot: RCSnapshot;
  recorded: RCCheckpointRecord[];
  terminal: { generationKey: string; reference: string }[];
  sourceRoot: string;
  manifestPath: string;
  archives: { target: string; path: string }[];
  startedAtMs: number;
  publishedAtMs: number;
  expiresAtMs: number;
}
export interface RCPublicationResult {
  mode: 'unarmed';
  status: 'refused' | 'frozen' | 'readback-confirmed';
  reason?: string;
  charges: RCPlan['charges'];
  recovery: RCCheckpointRecovery | RCPublicationRecovery | null;
  checkpoint: RCCheckpointResult | null;
  observations?: { kind: string; artifactId: number | null; response: RCPublicationResponse }[];
}
export function coordinatePRRC(
  input: RCPublicationInput,
  ports: RCPublicationPorts
): Promise<RCPublicationResult>;
