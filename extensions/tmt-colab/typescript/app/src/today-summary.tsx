import type { HomeSummary } from './bootstrap.js';
import { pageUpdate } from './page-index.js';
import { text } from './strings.js';
import { pagesChangedSince } from './today-model.js';
import type { PageSummary } from './transport.js';

const t = text.today;

/** The maintainer agent's two lines under the greeting, with who wrote them and when. A summary
 * older than the newest page change stays, with a muted note: it never claims nothing changed. */
export function TodaySummary({
  summary,
  pages,
  now,
}: {
  summary: HomeSummary;
  pages: readonly PageSummary[];
  now: number;
}) {
  const written = pageUpdate(summary.atMs, now);
  return (
    <section className="today-summary" aria-label={t.summaryLabel}>
      <div className="today-summary-lines">
        {summary.lines.map((line, index) => (
          <p key={index} className="today-summary-line">
            {line}
          </p>
        ))}
      </div>
      <p className="today-summary-meta">
        {summary.author ? `${summary.author} · ` : null}
        {written.dateTime ? (
          <time dateTime={written.dateTime} title={written.absolute}>
            {written.label}
          </time>
        ) : (
          written.label
        )}
      </p>
      {pagesChangedSince(summary, pages) && (
        <p className="today-summary-note">{t.summaryChanged}</p>
      )}
    </section>
  );
}
