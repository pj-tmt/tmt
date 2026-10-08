import type { ContentPreparation, ContentSnapshot, FoldResult } from '../src/fold-protocol.js';

/** The detached snapshot a content preparation is based on, from a decoder result. */
export function contentBase(result: FoldResult): ContentSnapshot {
  return {
    source: result.source,
    title: result.title,
    own: result.own,
    ...(Object.hasOwn(result, 'attachments') ? { attachments: result.attachments } : {}),
    ...(result.publisherAgent !== undefined ? { publisherAgent: result.publisherAgent } : {}),
    ...(Object.hasOwn(result, 'creationRecipient')
      ? { creationRecipient: result.creationRecipient }
      : {}),
  };
}
/** The deltas of a preparation that changed the source. */
export function updatesOf(prepared: ContentPreparation): Uint8Array[] {
  if (prepared.kind !== 'updates') throw new Error('Expected content updates');
  return prepared.updates;
}
