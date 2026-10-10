import type { QueueReader } from './pr-title-check.mjs';
export const AGENT_PRESETS: readonly string[];
export function parseAgentLines(body: unknown): { agents: string[]; findings: string[] };
export function checkPrAgents(input: {
  eventName: string;
  event: unknown;
  reader: QueueReader & { rest(endpoint: string): string };
  repository: string;
  reportOnly?: boolean;
}): { checked: number; exempted: number[]; findings: { number: number; findings: string[] }[] };
