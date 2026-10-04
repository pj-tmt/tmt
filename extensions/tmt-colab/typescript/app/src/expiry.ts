/** Server-observed local metadata is a display hint, never an access decision. */
export interface ExpiryInfo {
  retentionDays: number | null;
  lastUpdateAtMs: number | null;
  expiresAtMs: number | null;
  warnings: string[];
}
export function utcTime(value: number | null): string {
  if (value === null) return 'Not recorded yet';
  const date = new Date(value);
  return Number.isFinite(date.getTime())
    ? date.toISOString().replace('T', ' ').replace('Z', ' UTC')
    : 'Date is beyond the supported range';
}
export function expiryText(page: Partial<ExpiryInfo>): string {
  if (page.retentionDays === null) return 'No expiry: kept forever.';
  if (page.warnings?.includes('expiry-out-of-range'))
    return 'Expiry date is beyond the supported range.';
  if (page.expiresAtMs == null) return 'Expiry starts after the next edit.';
  const date = utcTime(page.expiresAtMs);
  if (page.warnings?.includes('expired'))
    return `Expired ${date}. Advisory only: this page is still available.`;
  if (page.warnings?.includes('expires-soon'))
    return `Expires ${date}, within seven days. Local data is never automatically deleted.`;
  return `Expires ${date}. Local data is never automatically deleted.`;
}
