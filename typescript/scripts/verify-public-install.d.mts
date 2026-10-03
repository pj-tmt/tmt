export interface SmokeResult {
  readonly check: string;
  readonly ok: boolean;
  readonly reason: string;
  /** Bounded multiline diagnostics, when the short reason omits command output. */
  readonly detail?: string;
  readonly infrastructure?: 'github-api-rate-limit';
  readonly rateLimit?: { readonly diagnostic: string; readonly resetAtMs: number | null };
}
export function parseRateLimitDiagnostic(diagnostic: unknown): SmokeResult['rateLimit'] | null;
export function installerUrl(repository: string): string;
export function smokeRelease(input: {
  product: string;
  tag: string;
  /** A checkout of the tag: only its data (the skills) is read. */
  source: string;
  repository: string;
  /** An empty directory the isolated home, state and prefix are made under. */
  root: string;
  /** The one network read: the text at a URL. */
  fetch?: (url: string) => Promise<string>;
  /** Unauthenticated bounded public asset download. */
  download?: (url: string, maximum: number) => Promise<Uint8Array>;
  target?: string;
  wait?: (milliseconds: number) => Promise<void>;
  now?: () => number;
  /** A deferred re-proof gets one acquisition attempt and cannot schedule another retry. */
  retry?: boolean;
  /** The directories after the prefix on the isolated PATH; the system's by default. */
  systemPath?: readonly string[];
}): Promise<SmokeResult[]>;
export function renderSmokeSummary(input: {
  tag: string;
  target: string;
  results: readonly SmokeResult[];
}): string;
