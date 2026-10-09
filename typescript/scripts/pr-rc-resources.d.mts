/** Planning data, not Core wire types or authenticated API receipts. */
export interface RCProducer {
  workflowId: number;
  workflowPath: string;
  workflowSha256: string;
  toolingCommit: string;
}
export interface RCEpoch {
  labelId: number;
  eventId: number;
  enabledAtMs: number;
}
export interface RCIdentity {
  repository: string;
  pr: number;
  sourceSha: string;
  epoch: RCEpoch;
  producer: RCProducer;
  runId: number;
  attempt: number;
  selection: { product: string; target: string }[];
}
export interface RCResource {
  /** Exact reserved output path in this run/attempt, never a discovery prefix. */
  path: string;
  kind: 'payload' | 'intermediate' | 'catalog' | 'diagnostic';
  /** One payload per canonical selected pair; other kinds use null. */
  selectionIndex: number | null;
  transportBytes: number;
  metadataBytes: number;
  manifestBytes: number;
  diagnosticBytes: number;
  artifactId: number | null;
}
export interface RCObservation {
  generationKey: string;
  reference: string;
  state: 'confirmed' | 'unknown';
  artifactIds: number[];
}
export interface RCGeneration {
  identity: RCIdentity;
  disposition: 'current' | 'pending' | 'retired';
  mutation: 'none' | 'unknown';
  resources: RCResource[];
  /** Read-only upstream provenance; never deletion ownership. */
  reuse: {
    repository: string;
    runId: number;
    attempt: number;
    artifactId: number;
    sha256: string;
  }[];
  observations: Record<'reservation' | 'settlement' | 'inventory' | 'absence', RCObservation>;
}
export interface RCSnapshot {
  repository: string;
  reference: string;
  complete: boolean;
  approvedProducer: RCProducer;
  channels: {
    pr: number;
    sourceRepository: string;
    sourceSha: string;
    epoch: RCEpoch;
    state: 'enabled' | 'disabled' | 'closed' | 'merged';
    reference: string;
  }[];
  generations: RCGeneration[];
  unknown: { reference: string; bytes: number; artifacts: number }[];
  journal: { checkpointBytes: number; checkpoints: number; terminalEntries: number };
}
export type RCRequest = { kind: 'reserve'; generation: RCGeneration } | { kind: 'reconcile' };
export interface RCIntent {
  kind: 'reserve' | 'upload' | 'retire-discovery' | 'cancel' | 'delete' | 'release-reservation';
  generationKey: string;
  requires: string[];
  bytes?: number;
  artifacts?: number;
  path?: string;
  artifactId?: number;
  runId?: number;
  attempt?: number;
}
export interface RCPlan {
  mode: 'unarmed';
  status: 'planned' | 'blocked' | 'refused';
  reason?: string;
  /** Original reservations remain charged even after a conditional release intent. Null is unknown, never zero. */
  charges: { bytes: number; artifacts: number } | null;
  intents: RCIntent[];
  blocked: { generationKey: string | null; obligations: string[] }[];
  evidence: string[];
}
export const RC_RESOURCE_LIMITS: Readonly<{
  channels: number;
  incomplete: number;
  generationBytes: number;
  generationArtifacts: number;
  aggregateBytes: number;
  manifestBytes: number;
  diagnosticBytes: number;
  checkpointBytes: number;
  checkpoints: number;
  terminalEntries: number;
  channelRecords: number;
  inventoryRecords: number;
  closeMs: number;
  retentionMs: number;
  reconciliationRequests: number;
  reconciliationBytes: number;
}>;
export function rcGenerationKey(identity: RCIdentity): string;
export function planRCResources(snapshot: RCSnapshot, request: RCRequest): RCPlan;
