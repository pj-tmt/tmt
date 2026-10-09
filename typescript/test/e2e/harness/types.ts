export interface CliResult<T = unknown> {
  code: number;
  stdout: string;
  stderr: string;
  json?: T;
}

export interface MockEvent {
  event:
    | 'ready'
    | 'request'
    | 'submitted'
    | 'summary'
    | 'failure'
    | 'fake-marker'
    | 'child-start'
    | 'child-close'
    | 'silent'
    | 'malformed'
    | 'input'
    | 'stopped';
  message?: string;
  line?: string;
  requestId?: string;
  receipt?: string;
  replyFrame?: string;
  body?: string;
  bodyBytes?: number;
  submittedAtMs?: number;
  stage?: string;
  exitCode?: number;
  childPid?: number;
  replyInput?: 'stdin' | 'message';
  error?: { code?: string; message?: string };
  mode?: string;
  pid?: number;
}

export interface MockPane {
  pane: string;
  pid: number;
  workspace: string;
}

export interface CliRunOptions {
  cwd?: string;
  /** Exercise client output handling independently of the fixture locale. */
  locale?: 'C' | 'C.UTF-8';
  pane?: string;
  /**
   * Remove pane context and fail visibly if the CLI attempts to invoke tmux. The
   * shim refuses every call, so tmux is also unreachable to notification and
   * transport: use `outsideTmux` to run outside a pane while tmux stays reachable.
   */
  withoutTmux?: boolean;
  /** Remove caller context while keeping tmux available for explicit targets. */
  outsideTmux?: boolean;
  /** Narrow caller-context overrides for invalid-evidence scenarios. */
  caller?: {
    tmux?: string | null;
    pane?: string | null;
  };
  /** Touch this file when the child has made its first tmux invocation. */
  progressFile?: string;
  /** Record transport sequencing without injecting a failure. */
  transportTrace?: boolean;
  /** Fail only diagnostic capture, after target resolution has succeeded. */
  captureFault?: 'exit' | 'overflow' | 'timeout';
  /** Inject one transport-stage failure into this CLI process only. */
  transportFault?: {
    /** set-buffer is safe to fall back from; paste and submit are uncertain. */
    readonly stage: 'set-buffer' | 'paste' | 'submit';
  };
}

export interface CliProcess<T = unknown> {
  readonly pid: number;
  readonly result: Promise<CliResult<T>>;
  /** Kill the CLI and descendants, including a paused tmux wrapper. */
  kill(signal?: NodeJS.Signals): void;
}

export interface MetadataBarrierOptions {
  /** Pause before or after the real durable metadata set-option. */
  readonly phase: 'before' | 'after';
  /** Publication is the default; clear pauses durable metadata removal. */
  readonly operation?: 'publish' | 'clear';
}

export interface E2EFixtureOptions {
  /** Test-only selector environment, resolved before allocating fixture resources. */
  executableEnv?: NodeJS.ProcessEnv;
  mode?: 'respond' | 'silent' | 'malformed' | 'virtualized' | 'fake-marker' | 'input-log';
  delayMs?: number;
  responseBodyBase64?: string;
  responseBytes?: number;
  responseMultibyte?: boolean;
  replyInput?: 'stdin' | 'message';
  replyDelayMs?: number;
  replyGate?: boolean;
  replyFailure?: boolean;
  holdReplyEof?: boolean;
  replyRetry?: boolean;
  replyConflict?: boolean;
  replyAckLoss?: boolean;
  summaryFailure?: boolean;
  globalDir?: string;
  metadataBarrier?: MetadataBarrierOptions;
}
