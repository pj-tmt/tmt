export function reconcilePublishedTags(
  tags: readonly string[],
  git: (args: string[], input?: string) => string
): void;
