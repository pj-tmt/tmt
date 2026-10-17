import { Fragment } from 'react';
import type { HomeSummary } from './bootstrap.js';
import { pageUpdate } from './page-index.js';
import { text } from './strings.js';
import { pagesChangedSince } from './today-model.js';
import type { PageSummary } from './transport.js';

const t = text.today;

/** The maintainer agent's two lines under the greeting, with who wrote them and when. A summary
 * older than the newest page change stays, with one more muted segment on its meta line: it never
 * claims nothing changed. Each segment keeps together, so a narrow screen wraps only at a `·`. */
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
        {[
          summary.author ? <span key="author">{summary.author}</span> : null,
          written.dateTime ? (
            <time key="written" dateTime={written.dateTime} title={written.absolute}>
              {written.label}
            </time>
          ) : (
            <span key="written">{written.label}</span>
          ),
          pagesChangedSince(summary, pages) ? <span key="changed">{t.summaryChanged}</span> : null,
        ]
          .filter((part) => part !== null)
          .map((part, index) => (
            <Fragment key={part.key}>
              {index > 0 && ' · '}
              {part}
            </Fragment>
          ))}
      </p>
    </section>
  );
}
