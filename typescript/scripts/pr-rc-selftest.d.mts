import type { RCPayload } from './pr-rc-coordinator.mjs';
export const QUALIFICATION: Readonly<{ pr: number; branch: string; workflow: number }>;
interface Fixture {
  head: string;
  run: number;
  catalog: number;
  payloads: readonly number[];
  label: number;
  event: number;
}
export function producerRunInput(value: unknown): number;
export function waitForPull(
  read: () => { data: Pull } | Promise<{ data: Pull }>,
  check: (pull: Pull) => void,
  sleep?: (ms: number) => void | Promise<void>
): Promise<Pull>;
export function deriveFixture(
  run: number,
  producer: unknown,
  catalogArtifact: unknown,
  catalog: unknown,
  inventory: unknown[],
  pull: Pull,
  timeline: unknown[]
): Fixture;
interface Pull {
  number: number;
  base: { repo: { full_name: string } };
  head: { repo: { full_name: string }; ref: string; sha: string };
  state: string;
  labels: { name: string; id?: number }[];
}
interface Candidate {
  target: string;
  version: string;
  archive: { sha256: string };
  payload_artifact: Pick<RCPayload, 'id'>;
}
interface Catalog {
  candidates: Candidate[];
}
interface Receipt {
  channel: string;
  version: string;
  archive_sha256: string;
  source: {
    kind: string;
    evidence: {
      repository: string;
      pr: number;
      head_sha: string;
      producer: { workflow_id: number; run_id: number };
      catalog_artifact_id: number;
      eligibility: { label_id: number; enabled_event_id: number };
      candidate: Candidate;
    };
  };
}
interface Artifact {
  id: number;
  name: string;
  size_in_bytes: number;
  digest: string;
}
export function safeEvidence(value: unknown): string;
export function assertOwnedPull(pull: Pull): void;
export function assertRefusal(
  result: { status: number | null; stdout: string; stderr: string },
  before: unknown,
  after: unknown
): void;
export function assertPositive(
  result: { status: number | null },
  before: unknown,
  after: unknown
): void;
export function assertPRReceipt(receipt: Receipt, catalog: Catalog, fixture: Fixture): void;
export function cleanupExact(
  manifest: { run: number; fixture: Fixture; rc: Artifact[]; ordinary: number[] },
  call: (
    method: string,
    route: string,
    data?: unknown,
    absent?: boolean
  ) => { status: number; data?: unknown } | Promise<{ status: number; data?: unknown }>,
  pull: Pull
): Promise<{
  retained: boolean;
  reason: string;
  artifacts: { id: number; alreadyAbsent: boolean }[];
}>;
export function readOwnedReceipt(world: string): Receipt;
export function executeSelftest(stage: string): Promise<void>;
