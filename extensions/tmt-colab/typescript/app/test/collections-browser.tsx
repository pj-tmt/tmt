/** Collection metadata scenarios on the existing page/action fixture. No production Store. */
import type { CollectionChange, CollectionPort } from '../src/collection-client.js';
import { CollectionError } from '../src/collection-client.js';
import type { CollectionDetail } from '../src/collections.js';
import type { PageSummary } from '../src/transport.js';
import { id } from './ask-fixtures.js';
import * as page from './ask-page-browser.js';
export const manualId = id(700),
  smartId = id(701);
let details: Map<string, CollectionDetail>;
let changes: CollectionChange[] = [];
let reads: string[] = [];
let refusal: 'refused' | 'unknown' | undefined;
export function proof() {
  return {
    changes: structuredClone(changes),
    reads: [...reads],
    records: page.discussionProof(),
    actions: page.proof().actions,
    sends: page.proof().sends,
    chapter: page.chapterProof(),
  };
}
export function refuse(mode: 'refused' | 'unknown') {
  refusal = mode;
}
export async function mount(
  kind:
    | 'manual'
    | 'directory'
    | 'empty-directory'
    | 'empty-manual'
    | 'smart-empty'
    | 'smart-pending'
    | 'smart-partial'
    | 'reading'
    | 'reading-busy'
    | 'review'
    | 'question' = 'manual',
) {
  changes = [];
  reads = [];
  refusal = undefined;
  details = new Map();
  const manual: CollectionDetail = {
    collection: {
      id: manualId,
      name: 'Release reading',
      maintainer: 'Agent 1',
      archived: false,
      definition: { kind: 'manual', sort: 'manual' },
    },
    items:
      kind === 'empty-manual'
        ? []
        : [
            {
              id: id(710),
              pageId: id(1),
              ...(kind.startsWith('reading') || kind === 'review' || kind === 'question'
                ? {}
                : { anchor: 'part:2' }),
              note: 'Review the figures before the meeting',
              audience: 'Ben',
            },
            {
              id: id(711),
              pageId: id(1),
              anchor: 'missing',
              note: 'This reference remains visible',
              audience: '',
            },
            { id: id(712), pageId: id(99), note: 'The page was removed', audience: 'Ben' },
          ],
    seen: [],
  };
  const smart: CollectionDetail = {
    collection: {
      id: smartId,
      name: 'Needs attention',
      maintainer: 'PM',
      archived: false,
      definition: { kind: 'smart', search: {}, needsYou: true },
    },
    items: [],
    seen: [],
  };
  if (kind !== 'empty-directory') {
    details.set(manualId, manual);
    details.set(smartId, smart);
  }
  const unknown = kind === 'smart-pending' || kind === 'smart-partial';
  const pages: PageSummary[] =
    kind === 'smart-empty'
      ? []
      : [
          ...(kind === 'smart-pending'
            ? []
            : [
                {
                  id: id(1),
                  title: 'Release plan',
                  sharing: 'private' as const,
                  lastUpdateAtMs: Date.now(),
                  summary: { comments: 2, waiting: true, needsYou: 2, author: 'Agent 1' },
                },
              ]),
          ...(unknown
            ? [
                {
                  id: id(98),
                  title: 'Another page',
                  sharing: 'private' as const,
                  lastUpdateAtMs: null,
                },
              ]
            : []),
        ];
  const port: CollectionPort = {
    async directory(signal) {
      signal?.throwIfAborted();
      reads.push('directory');
      return structuredClone([...details.values()].map((d) => d.collection));
    },
    async read(id, signal) {
      signal?.throwIfAborted();
      reads.push(id);
      const detail = details.get(id);
      if (!detail) throw new CollectionError('COLAB_COLLECTION_NOT_FOUND');
      return structuredClone(detail);
    },
    async change(command, signal) {
      signal?.throwIfAborted();
      changes.push(structuredClone(command));
      if (refusal)
        throw new CollectionError(refusal === 'unknown' ? 'COLAB_OUTCOME_UNKNOWN' : 'DENIED');
      const d = details.get(command.id)!;
      switch (command.op) {
        case 'seen':
          if (!d.seen.some((s) => s.id === command.target))
            d.seen = [...d.seen, { id: command.target, atMs: Date.now() }];
          break;
        case 'clear-seen':
          d.seen = [];
          break;
        case 'rename':
          d.collection.name = command.name;
          break;
        case 'maintainer':
          d.collection.maintainer = command.maintainer;
          break;
        case 'archive':
          d.collection.archived = command.archived;
          break;
        case 'sort':
          if (d.collection.definition.kind === 'manual')
            d.collection.definition.sort = command.sort;
          else d.collection.definition.search.sort = command.sort === 'title' ? 'title' : undefined;
          break;
      }
      return { detail: structuredClone(d), changed: true };
    },
  };
  const directory = kind === 'directory' || kind === 'empty-directory';
  await page.mount({
    todayPassages: true,
    statusAgents: true,
    holdStatusNotice: kind === 'reading-busy',
    collections: port,
    collectionPages: pages,
    collectionRoute: directory
      ? '/collections'
      : `/collections/${kind.startsWith('smart') ? smartId : manualId}`,
  });
  if (kind.startsWith('reading')) page.reading({ count: 2 });
  else if (kind === 'review') page.review({ sections: 2 });
  else if (kind === 'question') page.question({ kind: 'short' });
  else
    page.change(
      '<h2 id="part:2">Release figures</h2><p>The live passage follows this exact heading.</p><tmt-read data-id="00000000-0000-4000-8000-000000000240" data-agent="Agent 1" data-why="Review the plan"><h2 data-title>Release decision</h2><p>Read the release evidence.</p></tmt-read><tmt-read data-id="00000000-0000-4000-8000-000000000241" data-agent="Agent 1" data-why="Review the rollback"><h2 data-title>Rollback decision</h2><p>Read the rollback evidence.</p></tmt-read>',
    );
}
export function reading() {
  page.reading({ count: 2 });
}
export function review() {
  page.review({ sections: 2 });
}
export function change(source: string) {
  page.change(source);
}
export {
  block,
  pausePrepare,
  resumePrepare,
  finishStatusNotice,
  question,
  answerBatchProof,
  discussionProof,
} from './ask-page-browser.js';
