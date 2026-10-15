import type {
  SealedSyncPort,
  SyncCall,
  SyncInterest,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import { HostedInterests, type HostedFailure } from './hosted-interests.js';
import type { HostedPageReader } from './hosted-reader.js';
import type { PageView } from './transport.js';
import { ReadmitBudget, READMIT_DELAYS_MS, waitReadmitDelay } from './readmit.js';

export interface HostedBinding {
  port: SealedSyncPort;
  interests: readonly SyncInterest[];
  reader: Pick<HostedPageReader, 'refresh' | 'close'>;
}
interface Options {
  /** Each attempt constructs fresh verification and subscriptions, never a dead interest. */
  open(call: SyncCall): Promise<HostedBinding>;
  drain(): Promise<void>;
  publish(view: PageView, call: SyncCall): void | Promise<void>;
  failed(failure: HostedFailure): void;
  recovering?(): void;
  wait?: typeof waitReadmitDelay;
}
const RECOVERY_MS = 30_000;

/** A tab's one hosted page. The entry must await close before selecting its successor. */
export class HostedSession {
  readonly ready: Promise<void>;
  private readonly controller = new AbortController();
  private readonly signal: AbortSignal;
  private readonly reopens = new ReadmitBudget();
  private readonly work: Promise<void>;
  private resolveReady!: () => void;
  private rejectReady!: (failure: unknown) => void;
  constructor(
    private readonly options: Options,
    call: SyncCall,
  ) {
    this.signal = AbortSignal.any([this.controller.signal, call.signal]);
    this.ready = new Promise((resolve, reject) => {
      this.resolveReady = resolve;
      this.rejectReady = reject;
    });
    this.work = this.run(call.deadlineAtMs);
  }
  /** Fence effects immediately, then join reads/listeners/charges before any new page opens. */
  async close(): Promise<void> {
    this.controller.abort();
    await this.work;
  }
  private async run(initialDeadline: number): Promise<void> {
    let deadlineAtMs = initialDeadline,
      attempts = 0;
    try {
      for (;;) {
        this.live(deadlineAtMs);
        let failure: HostedFailure;
        try {
          const call = { signal: this.signal, deadlineAtMs },
            binding = await this.options.open(call);
          let drop!: (failure: HostedFailure) => void;
          const dropped = new Promise<HostedFailure>((resolve) => {
            drop = resolve;
          });
          const abort = () => drop(cancelled());
          this.signal.addEventListener('abort', abort, { once: true });
          const group = new HostedInterests(binding.port, this.options.drain, drop);
          try {
            this.live(deadlineAtMs);
            await group.replace(
              binding.interests,
              async (call, resources) => {
                const view = await binding.reader.refresh(call, resources);
                if (!call.signal.aborted && !this.signal.aborted)
                  await this.options.publish(view, call);
              },
              call,
            );
            this.resolveReady();
            // A healthy interest has no lifetime timer. Only the next recovery run gets a bound.
            attempts = 0;
            failure = await dropped;
            deadlineAtMs = Date.now() + RECOVERY_MS;
          } finally {
            try {
              await group.close();
            } finally {
              this.signal.removeEventListener('abort', abort);
              try {
                binding.reader.close();
              } finally {
                binding.port.close();
              }
            }
          }
        } catch (error) {
          await this.options.drain();
          failure = error as HostedFailure;
        }
        if (!recoverable(failure) && !isCancellation(failure)) throw failure;
        if (this.signal.aborted) throw cancelled();
        if (
          !recoverable(failure) ||
          ++attempts >= 3 ||
          Date.now() >= deadlineAtMs ||
          !this.reopens.take(Date.now())
        )
          throw failure;
        this.options.recovering?.();
        await (this.options.wait ?? waitReadmitDelay)(
          Math.min(READMIT_DELAYS_MS[attempts - 1] ?? 3000, Math.max(deadlineAtMs - Date.now(), 0)),
          this.signal,
        );
      }
    } catch (error) {
      this.rejectReady(error);
      // A terminal refusal observed before a hidden/close event cannot become a fresh retry.
      if (!this.signal.aborted || (!recoverable(error as HostedFailure) && !isCancellation(error)))
        this.options.failed(error as HostedFailure);
    }
  }
  private live(deadlineAtMs: number): void {
    if (this.signal.aborted) throw cancelled();
    if (!Number.isFinite(deadlineAtMs) || Date.now() >= deadlineAtMs)
      throw { code: 'REMOTE_STATE_UNAVAILABLE', state: 'deadline' };
  }
}

function recoverable(failure: HostedFailure): boolean {
  return (
    failure !== null &&
    typeof failure === 'object' &&
    'code' in failure &&
    failure.code === 'REMOTE_STATE_UNAVAILABLE' &&
    (!('state' in failure) || failure.state === 'catch-up-required')
  );
}
const cancelled = (): HostedFailure => ({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' });
function isCancellation(failure: unknown): boolean {
  return (
    typeof failure === 'object' &&
    failure !== null &&
    'code' in failure &&
    failure.code === 'REMOTE_STATE_UNAVAILABLE' &&
    'state' in failure &&
    failure.state === 'cancelled'
  );
}
