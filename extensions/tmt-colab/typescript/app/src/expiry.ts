/** Server-observed metadata is a display hint, never an access decision. */
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
  // A page kept forever locally still has a cloud copy with a confirmed deadline, and losing it
  // ends sharing; with no confirmed row there is nothing more to say than kept forever.
  if (page.retentionDays === null) {
    if (page.expiresAtMs == null) return 'kept forever';
    const cloud = cloudExpiry(page.expiresAtMs, now);
    return `kept forever here · ${cloud ? `cloud copy ${cloud}` : 'not in the cloud'}`;
  }
  // The expiry is the cloud copy's: what the last confirmed write of the page's row made it.
  // A page this machine never put in the cloud has none and is only local.
  if (page.expiresAtMs == null)
    return page.warnings?.includes('not-in-cloud') ? 'not in the cloud' : 'local only';
  return cloudExpiry(page.expiresAtMs, now) ?? 'not in the cloud';
}
/** `expires in …` while the cloud copy lasts; nothing once its confirmed expiry has passed. */
function cloudExpiry(expiresAtMs: number, now: number): string | undefined {
  const remaining = expiresAtMs - now;
  if (remaining <= 0) return undefined;
  const days = Math.floor(remaining / 86400000);
  const hours = Math.floor(remaining / 3600000);
  const interval =
    days >= 1
      ? `${days} ${days === 1 ? 'day' : 'days'}`
      : hours >= 1
        ? `${hours} h`
        : 'less than an hour';
  return `expires in ${interval}`;
}
