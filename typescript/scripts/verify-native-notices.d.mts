import type { runPackedCommand } from './packed-command.mjs';

export interface NoticeVerification {
  readonly product: string;
  readonly target: string;
  readonly seconds: number;
}

export function releaseNoticeTargets(root: string, runner?: typeof runPackedCommand): string[];
export function verifyNativeNotices(
  root: string,
  options?: { runner?: typeof runPackedCommand; report?: (message: string) => void }
): NoticeVerification[];
