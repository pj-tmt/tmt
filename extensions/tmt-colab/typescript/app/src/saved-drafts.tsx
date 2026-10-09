import { BrowserAction } from '@tmt/browser-ui/react';
import type { ComposerEdit } from './components/message-composer-edit.js';
import { persistable } from './draft-store.js';
import { text } from './strings.js';
import { validateSelector, type ThreadView } from './thread-records.js';

/** The reserved draft key of the page Chat composer. */
export const CHAT_DRAFT = 'chat';

export interface SavedDraft {
  key: string;
  /** The quote of an unsent selection draft; absent for a thread whose record is gone. */
  quote?: string;
  text: string;
}

/**
 * Drafts the page cannot show in place: an unsent selection (the quote must be selected
 * again) and a reply to a thread that no longer exists. A draft for a live thread is shown
 * by that thread, and Chat shows its own. Nothing here sends, re-anchors or deletes.
 */
export function savedDrafts(
  drafts: ReadonlyMap<string, ComposerEdit>,
  threads: readonly ThreadView[] | undefined,
): SavedDraft[] {
  const live = new Set(
    (threads ?? []).filter((t) => !t.deleted).map((t) => `${t.ref.writer}:${t.ref.id}`),
  );
  const result: SavedDraft[] = [];
  for (const [key, edit] of drafts) {
    if (key === CHAT_DRAFT || !persistable(edit)) continue;
    if (key.startsWith('{')) {
      try {
        const selector: unknown = JSON.parse(key);
        validateSelector(selector);
        result.push({ key, quote: selector.exact, text: edit.value });
      } catch {
        // A key that is not a selector is not listed as one.
      }
    } else if (!live.has(key)) result.push({ key, text: edit.value });
  }
  return result;
}

export function SavedDrafts({
  drafts,
  discard,
}: {
  drafts: readonly SavedDraft[];
  discard(key: string): void;
}) {
  if (!drafts.length) return null;
  return (
    <section className="saved-drafts" data-testid="saved-drafts" aria-label={text.savedDrafts}>
      <h3>{text.savedDrafts}</h3>
      <ul>
        {drafts.map((draft) => (
          <li key={draft.key} data-testid="saved-draft">
            {draft.quote !== undefined && <blockquote>{draft.quote}</blockquote>}
            <p className="saved-draft-text">{draft.text}</p>
            <p className="annotation-hint">
              {draft.quote !== undefined ? text.savedDraftQuote : text.savedDraftThreadGone}
            </p>
            <BrowserAction
              type="button"
              label={text.savedDraftDiscard}
              variant="text"
              onActivate={(event) => {
                if (event.isTrusted) discard(draft.key);
              }}
            />
          </li>
        ))}
      </ul>
    </section>
  );
}
