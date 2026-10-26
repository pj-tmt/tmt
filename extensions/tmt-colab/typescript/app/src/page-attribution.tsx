import { AgentChip } from './components/agent-chip.js';
import type { Projection } from './fold-protocol.js';
import { text } from './strings.js';

/** Display-only legacy fallback: a stored recipient ID is never a human label. */
export function creatorName({
  creator,
  creationRecipient,
  originalAuthor,
}: Pick<Projection, 'creator' | 'creationRecipient' | 'originalAuthor'>): string | undefined {
  return (
    creator?.agent.name ?? (creationRecipient ? (originalAuthor ?? text.unknownAuthor) : undefined)
  );
}

/** Canonical display labels only; never identity or recipient authority. */
export function PageAttribution({
  originalAuthor,
  creator,
  creationRecipient,
  publisherAgent,
}: Pick<Projection, 'originalAuthor' | 'publisherAgent' | 'creator' | 'creationRecipient'>) {
  return (
    <>
      <dl className="page-attribution">
        <dt>{text.metadata.creator}</dt>
        <dd>
          {creatorName({ creator, creationRecipient, originalAuthor }) ?? text.metadata.noCreator}
        </dd>
        <dt>{text.originalAuthor}</dt>
        <dd>{originalAuthor ?? text.unknownAuthor}</dd>
        {publisherAgent !== undefined && (
          <>
            <dt>{text.latestPublisher}</dt>
            <dd>
              <AgentChip name={publisherAgent} />
            </dd>
          </>
        )}
      </dl>
      <p className="page-attribution-note">{text.authorAttributionNote}</p>
    </>
  );
}
