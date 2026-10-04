/** Server-observed local metadata is a display hint, never an access decision. */
export interface ExpiryInfo {
  retentionDays: number | null;
  lastUpdateAtMs: number | null;
  expiresAtMs: number | null;
  warnings: string[];
}
/** The browser's local absolute date is secondary hover information, without milliseconds. */
export function localTime(value: number | null): string {
  if (value === null) return 'Not recorded yet';
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return 'Date is beyond the supported range';
  const parts = new Intl.DateTimeFormat('en-US', {
    weekday: 'short',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).formatToParts(date);
  const part = (type: Intl.DateTimeFormatPartTypes) => parts.find((p) => p.type === type)?.value;
  return `${part('weekday')} ${part('month')}-${part('day')} ${part('hour')}:${part('minute')}`;
}
export function expiryText(page: Partial<ExpiryInfo>, now = Date.now()): string {
  if (page.retentionDays === null) return 'Kept forever.';
  if (page.warnings?.includes('expiry-out-of-range'))
    return 'Retention date is beyond the supported range.';
  if (page.expiresAtMs == null) return 'Expiry starts after the next edit.';
  const remaining = page.expiresAtMs - now;
  const duration = Math.abs(remaining);
  const days = Math.floor(duration / 86400000);
  const hours = Math.floor(duration / 3600000);
  const interval =
    days >= 1
      ? `${days} ${days === 1 ? 'day' : 'days'}`
      : hours >= 1
        ? `${hours} h`
        : 'less than an hour';
  const relative = remaining > 0 ? `ends in ${interval}` : `ended ${interval} ago`;
  return `Retention ${relative} · your local copy stays`;
}
