/**
 * One deadline for a long scenario, split into named stages.
 *
 * A scenario that waits on files or child processes inside the framework's own timeout reports
 * only "Test timed out" when a late stage stalls, and the stage that stalled is lost. A budget
 * owns the scenario's whole allowance: it gives every wait the time that is really left, never
 * more than the stage's own cap, keeps teardown time out of the waits, and names the stage that
 * ran out.
 */
export interface StageBudgetOptions {
  /** The framework timeout of the scenario this budget serves. */
  totalMs: number;
  /** Time kept back for teardown after the last stage. */
  cleanupReserveMs: number;
  now?: () => number;
}

interface StageRecord {
  name: string;
  startedAtMs: number;
  endedAtMs?: number;
  failed?: boolean;
}

export class StageBudget {
  private readonly now: () => number;
  private readonly startedAtMs: number;
  private readonly stages: StageRecord[] = [];

  constructor(private readonly options: StageBudgetOptions) {
    if (options.cleanupReserveMs >= options.totalMs)
      throw new Error('The cleanup reserve must be smaller than the scenario timeout.');
    this.now = options.now ?? Date.now;
    this.startedAtMs = this.now();
  }

  /** Time left for stages: the scenario timeout, minus what is spent and the cleanup reserve. */
  remainingMs(): number {
    return Math.max(
      0,
      this.startedAtMs + this.options.totalMs - this.options.cleanupReserveMs - this.now()
    );
  }

  /**
   * Runs one stage. `work` receives the allowance for its waits: the smaller of the stage's cap
   * and the time left. A stage that fails, or is given no time at all, fails with an error that
   * names it and the stages before it; the original error stays as the cause.
   */
  async stage<T>(
    name: string,
    work: (allowanceMs: number) => Promise<T> | T,
    { capMs = Infinity }: { capMs?: number } = {}
  ): Promise<T> {
    const allowanceMs = Math.min(capMs, this.remainingMs());
    const record: StageRecord = { name, startedAtMs: this.now() };
    this.stages.push(record);
    try {
      if (allowanceMs <= 0) throw new Error('The scenario had no time left for this stage.');
      const value = await work(allowanceMs);
      record.endedAtMs = this.now();
      return value;
    } catch (error) {
      record.endedAtMs = this.now();
      record.failed = true;
      const reason = error instanceof Error ? error.message : String(error);
      throw new Error(
        `Stage "${name}" failed after ${record.endedAtMs - record.startedAtMs} ms of its ` +
          `${Math.floor(allowanceMs)} ms allowance. ${this.evidence()}\n${reason}`,
        { cause: error }
      );
    }
  }

  /** What each stage spent so far, including a stage that is still running. */
  evidence(): string {
    const now = this.now();
    const stages = this.stages
      .map((stage) => {
        if (stage.endedAtMs === undefined)
          return `${stage.name} still running after ${now - stage.startedAtMs} ms`;
        const spent = stage.endedAtMs - stage.startedAtMs;
        return `${stage.name} ${spent} ms ${stage.failed ? 'FAILED' : 'ok'}`;
      })
      .join('; ');
    const spent = now - this.startedAtMs;
    return (
      `Stages: ${stages || 'none started'}. ` +
      `${spent} of ${this.options.totalMs} ms spent, ${this.options.cleanupReserveMs} ms reserved for cleanup.`
    );
  }
}
