export const MOCK_MODEL_TOKEN: 'tmt-synthetic-model-token';
export interface ScriptedTextTurn {
  protocol: 'messages' | 'responses';
  model: string;
  text: string;
  /** Optional exact last-user text; a mismatch cannot consume this turn. */
  inputText?: string;
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
  | { model?: string; response: JsonValue; turn?: never; unlessLastUserText?: never }
  | {
      turn: ScriptedTextTurn;
      response?: never;
      model?: never;
      /** At most two tool-free side calls with plain last-user text different from this input. */
      unlessLastUserText?: string;
    }
);
export interface MockInputMismatch {
  parsed: { state: 'text' | 'missing_or_unsupported'; bytes: number | null; sha256: string | null };
  scripted: { bytes: number; sha256: string };
  firstDifferingByte: number | null;
  /** UTF-8 excerpts total at most 512 bytes, with the synthetic credential redacted. */
  excerpts: { parsed: string; scripted: string };
}
export function describeMockInputMismatch(
  parsed: string | undefined,
  scripted: string
): MockInputMismatch;
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
    /** Validated UUID only; absent until the selected pinned wire field is present. */
    sessionId?: string;
    code?: string;
    kind?: 'health' | 'side' | 'turn';
  }[];
}
export function startMockModelServer(options: {
  turns: ScriptedTextTurn[];
  sideRequests?: DeclaredSideRequest[];
  /** Opt-in synthetic fixture evidence; ordinary snapshots never retain input text. */
  onInputMismatch?: (diagnostic: MockInputMismatch) => void;
  /** Wire locations are candidates until matched against hosted public CLI session evidence. */
  sessionSource?: 'claude-code-2.1.285' | 'codex-0.160.0';
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
