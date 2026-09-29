export function releaseMode(input: { event: string; dryRun?: string; hasSecrets: boolean }): {
  live: boolean;
  reason: string;
};
