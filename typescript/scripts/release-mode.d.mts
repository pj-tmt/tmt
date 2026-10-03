export function releaseMode(input: { event: string; ref?: string; dryRun?: string }): {
  live: boolean;
  reason: string;
};
