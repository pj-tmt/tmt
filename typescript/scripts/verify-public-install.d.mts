export interface SmokeResult {
  readonly check: string;
  readonly ok: boolean;
  readonly reason: string;
  /** Bounded multiline diagnostics, when the short reason omits command output. */
  readonly detail?: string;
}
/** How long, and how often, the public latest pointer may keep naming an older alpha. */
export const LATEST_LAG_DEADLINE_MS: number;
export const LATEST_LAG_POLL_MS: number;
export function installerUrl(repository: string, tag?: string): string;
export function smokeRelease(input: {
  product: string;
  tag: string;
  /** A checkout of the tag: only its data (the skills) is read. */
  source: string;
  repository: string;
  /** An empty directory the isolated home, state and prefix are made under. */
  root: string;
  /** The actual target expected before and after installation/upgrade. */
  target?: string;
  /** Architecture inspection only; orchestration fixtures supply their own byte oracle. */
  inspectArchitecture?: (
    executable: string,
    target: string,
    options: { cwd: string; env: Record<string, string> }
  ) => void;
  /** The one network read: the text at a URL. */
  fetch?: (url: string) => Promise<string>;
  /** Unauthenticated bounded public asset download. */
  download?: (url: string, maximum: number) => Promise<Uint8Array>;
  target?: string;
  wait?: (milliseconds: number) => Promise<void>;
  /** The clock, in milliseconds, that bounds the latest-installer lag wait. */
  now?: () => number;
  /** Read-only API credential, passed only in acquisition process environments. */
  githubToken?: string;
  /** The directories after the prefix on the isolated PATH; the system's by default. */
  systemPath?: readonly string[];
  /** Installed-app proof boundary, injected only by verifier fixture tests. */
  verifyColab?: typeof import('./colab-runtime-proof.mjs').verifyColabApp;
}): Promise<SmokeResult[]>;
export function renderSmokeSummary(input: {
  tag: string;
  target: string;
  results: readonly SmokeResult[];
}): string;
