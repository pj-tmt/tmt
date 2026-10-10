/**
 * A paired owner's open page ends with these refusals when the page epoch advances or an
 * authority recheck drops its sync socket (#2557). None says the device lost access: reopening the
 * page binds the new epoch, and a revoked device is refused by that fresh open, which is terminal.
 */
const READMITTABLE = ['STALE_EPOCH', 'DENIED', 'EXPIRED'] as const;

export const readmittable = (error: Error): boolean =>
  (READMITTABLE as readonly string[]).includes(error.message);

/** At most `limit` automatic reopens inside `windowMs`; a page that keeps failing then stops. */
export class ReadmitBudget {
  #times: number[] = [];
  constructor(
    private readonly limit = 3,
    private readonly windowMs = 60_000,
  ) {}
  take(now: number): boolean {
    this.#times = this.#times.filter((time) => now - time < this.windowMs);
    if (this.#times.length >= this.limit) return false;
    this.#times.push(now);
    return true;
  }
}
