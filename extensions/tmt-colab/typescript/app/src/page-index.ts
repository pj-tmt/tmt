import type { PageSummary } from './transport.js';
import { relativeTime } from './display-time.js';
import { text } from './strings.js';

export function pageTitle(page: PageSummary): string {
  return page.title.trim() || text.unknownPageTitle;
}

/** Display ordering only; the metadata does not grant page access. */
export function orderPages(pages: readonly PageSummary[]): PageSummary[] {
  return [...pages].sort(
    (a, b) =>
      (a.lastUpdateAtMs == null
        ? b.lastUpdateAtMs == null
          ? 0
          : 1
        : b.lastUpdateAtMs == null
          ? -1
          : b.lastUpdateAtMs - a.lastUpdateAtMs) ||
      pageTitle(a).localeCompare(pageTitle(b), 'en') ||
      a.id.localeCompare(b.id, 'en'),
  );
}

export function pageUpdate(value: number | null | undefined, now: number) {
  const date = value == null ? undefined : new Date(value);
  if (!date || !Number.isFinite(date.getTime())) return { label: text.updateTimeUnknown };
  return {
    label: relativeTime(date.getTime(), now),
    dateTime: date.toISOString(),
    absolute: new Intl.DateTimeFormat('en-US', {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
      timeZoneName: 'short',
    }).format(date),
  };
}
