export const RATCHET: string;
export interface PreviousRepositoryLinks {
  path: string;
  lines: number[];
}
export function linkLines(text: string): number[];
export function scan(root?: string): PreviousRepositoryLinks[];
export function readRatchet(root?: string): Record<string, number>;
export function violations(
  found: PreviousRepositoryLinks[],
  allowed: Record<string, number>
): string[];
