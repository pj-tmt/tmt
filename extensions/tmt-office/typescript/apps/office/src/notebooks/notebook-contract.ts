import { boundedText, canonicalUuid, exactRecord } from '../contracts/record.js';

export const NOTEBOOK_READ_BYTES = 1_048_576;
// JSON may escape every source byte as six ASCII bytes; allow bounded metadata too.
export const NOTEBOOK_ENVELOPE_BYTES = NOTEBOOK_READ_BYTES * 6 + 4096;

export interface Notebook {
  identityId: string;
  name: string;
  content: string;
}

export interface NotebookPort {
  read(identityId: string, signal?: AbortSignal): Promise<Notebook>;
}

export function decodeNotebook(value: unknown): Notebook {
  const input = exactRecord(value, ['identityId', 'name', 'content'], 'notebook');
  const identityId = canonicalUuid(input.identityId);
  if (
    !boundedText(input.name, 256) ||
    typeof input.content !== 'string' ||
    /[\uD800-\uDFFF]/u.test(input.content) ||
    new TextEncoder().encode(input.content).length > NOTEBOOK_READ_BYTES
  )
    throw new Error('Invalid notebook content.');
  return { identityId, name: input.name, content: input.content };
}
