export interface SmokeResult {
  readonly check: string;
  readonly ok: boolean;
  readonly reason: string;
  /** Bounded multiline diagnostics, when the short reason omits command output. */
  readonly detail?: string;
  readonly infrastructure?: 'github-api-rate-limit';
}
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
  wait?: (milliseconds: number) => Promise<void>;
  now?: () => number;
  /** The directories after the prefix on the isolated PATH; the system's by default. */
  systemPath?: readonly string[];
}): Promise<SmokeResult[]>;
export function renderSmokeSummary(input: {
  tag: string;
  target: string;
  results: readonly SmokeResult[];
}): string;
