import type { Projection } from './fold-protocol.js';
import { text } from './strings.js';

/** Canonical display labels only; never identity or recipient authority. */
export function PageAttribution({
  originalAuthor,
  publisherAgent,
}: Pick<Projection, 'originalAuthor' | 'publisherAgent'>) {
  return (
    <>
      <dl className="page-attribution">
        <dt>{text.originalAuthor}</dt>
        <dd>{originalAuthor ?? text.unknownAuthor}</dd>
        {publisherAgent !== undefined && (
          <>
            <dt>{text.latestPublisher}</dt>
            <dd>{publisherAgent}</dd>
          </>
        )}
      </dl>
      <p className="page-attribution-note">{text.authorAttributionNote}</p>
    </>
  );
}
