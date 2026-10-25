import { AgentChip } from './components/agent-chip.js';
import type { Projection } from './fold-protocol.js';
import { text } from './strings.js';

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
          {creator?.agent.name ??
            (creationRecipient
              ? (originalAuthor ?? creationRecipient.agentId)
              : text.metadata.noCreator)}
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
