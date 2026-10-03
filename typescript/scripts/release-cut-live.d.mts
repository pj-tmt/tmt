import type { CutMetadata, CutRow } from './release-cut.mjs';
export interface DraftRelease {
  id: number;
  draft: boolean;
  tag_name: string;
  target_commitish: string;
  body?: string;
  assets?: { name: string }[];
}
export interface CutClient {
  main(): string;
  metadata(cut: string): CutMetadata & { releases: DraftRelease[] };
  release(id: number): DraftRelease;
  tagged(tag: string): boolean;
  draft(input: { tag: string; cut: string; body: string; product: string }): DraftRelease;
  dispatch(product: string, tag: string): void;
}
export function createCutClient(
  input: { repository: string; token: string; ref: string },
  execute?: (
    executable: string,
    args: string[],
    options: { cwd: string; env: NodeJS.ProcessEnv; timeoutMs: number }
  ) => string
): CutClient;
export function cutDraftBody(row: CutRow): string;
export function runReleaseCuts(input: {
  client: CutClient;
  git: (args: string[]) => string;
  live?: boolean;
  date?: string;
  product?: string;
  version?: string;
  root?: string;
}): Promise<{
  cut: string;
  mode: string;
  components: CutRow[];
  actions: { product: string; status: string; reason?: string; tag?: string; cut?: string }[];
}>;
