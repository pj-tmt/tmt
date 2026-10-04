export type WorkflowJob = { steps: string[]; sha256: string; uses?: string; body: string };
export function inventoryWorkflow(
  text: string,
  file: string,
  selectedJob?: string
): Record<string, WorkflowJob>;
export function releaseInventory(
  read: (file: string) => string
): Record<string, Record<string, WorkflowJob>>;
export function checkReleaseParity(
  manifest: unknown,
  options: { read: (file: string) => string; publicationGates?: string[] }
): { workflows: number; jobs: number; publicationGates: number };
