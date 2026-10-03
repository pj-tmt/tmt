export function readCutMetadata(
  input: {
    repository: string;
    cut: string;
    token: string;
    draftVisibility: string;
  },
  execute?: (
    executable: string,
    args: string[],
    options: {
      cwd: string;
      env: NodeJS.ProcessEnv;
      timeoutMs: number;
    }
  ) => string
): {
  schema: number;
  repository: string;
  cut: string;
  draftVisibility: string;
  capturedAt: string;
  releases: {
    id: number;
    tag_name: string;
    draft: boolean;
    body?: string;
    target_commitish?: string;
  }[];
};
