import { useEffect, useState } from 'react';
import { digest, encodeBinary, text as utf8 } from '@tmt/colab-client';
import type { NeedsYouCandidate } from './needs-you.js';

/** Item versions bind authored content and admitted owner state, never cosmetic anchor context. */
export function useNeedsYouVersions(source: string, candidates: readonly NeedsYouCandidate[]) {
  const signature = JSON.stringify(
    candidates.map((item) => [
      item.kind,
      item.id,
      item.version ?? JSON.stringify([item.title, item.agents]),
    ]),
  );
  const [snapshot, setSnapshot] = useState<{
    source: string;
    signature: string;
    versions?: ReadonlyMap<string, string>;
  }>();
  useEffect(() => {
    let current = true;
    const targets = JSON.parse(signature) as [NeedsYouCandidate['kind'], string, string][];
    const wanted = new Set(targets.map(([kind, id]) => `${kind}:${id}`));
    const fragments = new Map<string, string | undefined>();
    if (wanted.size) {
      const template = document.createElement('template');
      // The same inert-template boundary as inspectQuestions: never attach authored nodes.
      template.innerHTML = source;
      const kinds: Record<string, NeedsYouCandidate['kind']> = {
        'tmt-question': 'answer',
        'tmt-questions': 'answer',
        'tmt-review': 'review',
        'tmt-read': 'reading',
        'tmt-proposal': 'proposal',
      };
      for (const node of template.content.querySelectorAll(
        'tmt-question,tmt-questions,tmt-review,tmt-read,tmt-proposal',
      )) {
        const id = node.getAttribute('data-id');
        if (!id) continue;
        const key = `${kinds[node.localName]}:${id}`;
        // Unlisted authored tags cannot expand version work or materialize nested HTML.
        if (!wanted.has(key)) continue;
        fragments.set(key, fragments.has(key) ? undefined : node.outerHTML);
      }
    }
    const versions = new Map<string, string>();
    void Promise.all(
      targets.map(async ([kind, id, owner]) => {
        const key = `${kind}:${id}`;
        const fragment = fragments.get(key);
        if (fragment !== undefined)
          versions.set(key, encodeBinary(await digest(utf8(JSON.stringify([fragment, owner])))));
      }),
    )
      .then(() => {
        if (current) setSnapshot({ source, signature, versions });
      })
      .catch(() => {
        if (current) setSnapshot({ source, signature });
      });
    return () => {
      current = false;
    };
  }, [source, signature]);
  const matching = snapshot?.source === source && snapshot.signature === signature;
  return {
    ready: matching && snapshot.versions !== undefined,
    failed: matching && snapshot.versions === undefined,
    items:
      matching && snapshot.versions
        ? candidates.flatMap((item) => {
            const version = snapshot.versions!.get(`${item.kind}:${item.id}`);
            return version ? [{ ...item, version }] : [];
          })
        : [],
  };
}
