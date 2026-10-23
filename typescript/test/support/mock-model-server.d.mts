export const MOCK_MODEL_TOKEN: 'tmt-synthetic-model-token';
export interface ScriptedTextTurn {
  protocol: 'messages' | 'responses';
  model: string;
  text: string;
  inputTokens: number;
  outputTokens: number;
  /** Messages cache reads are separate from input; Responses caches are a subset. */
  cachedInputTokens?: number;
}
export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };
export type DeclaredSideRequest = {
  method: 'GET' | 'HEAD' | 'POST';
  path: string;
  maxCalls: number;
} & (
  | { model?: string; response: JsonValue; turn?: never }
  | { turn: ScriptedTextTurn; response?: never; model?: never }
);
export interface MockModelSnapshot {
  requestsSeen: number;
  sideRequestsSeen: number;
  completedTurns: number;
  plannedTurns: number;
  closed: boolean;
  failure?: { code: string; message: string };
  census: {
    method: string;
    path: string;
    status: number;
    bodyBytes: number;
    model?: string;
    stream?: boolean;
    code?: string;
    kind?: 'health' | 'side' | 'turn';
  }[];
}
export function startMockModelServer(options: {
  turns: ScriptedTextTurn[];
  sideRequests?: DeclaredSideRequest[];
  limits?: {
    maxRequests?: number;
    maxBodyBytes?: number;
    timeoutMs?: number;
    requestTimeoutMs?: number;
  };
}): Promise<{
  baseUrl: string;
  close(): Promise<void>;
  closed: Promise<void>;
  snapshot(): MockModelSnapshot;
}>;
