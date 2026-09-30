export function releaseMode(input: {
  event: string;
  ref?: string;
  dryRun?: string;
  hasSecrets: boolean;
}): {
  live: boolean;
  reason: string;
};
