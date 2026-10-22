import { useCallback, useLayoutEffect, useRef, useState, type MutableRefObject } from 'react';
import { type PageSize } from './page-size.js';
import { compactOffered } from './page-usage.js';
import { SaveConflict, SaveOutcomeUnknown, SaveRefused, saveMessage } from './save.js';
import type { SaveProblem } from './save-notice.js';
import { text } from './strings.js';
import type { PageBinding, PageView } from './transport.js';

/** What the editor needs of the opened page. */
interface EditedPage {
  id: string;
  source: string;
  binding?: PageBinding;
}

/**
 * The Source draft of an open page and its Save: the text being edited, the source it was opened
 * from (`base`, frozen while the draft is dirty), how the last Save ended, and where the caret was
 * when a merged Save replaced the text. The page feeds it what arrives: a replaced snapshot
 * (`adoptSnapshot`) and each live update (`adoptUpdate`); a dirty draft keeps its frozen base and
 * caret, never saved or retargeted, and a clean one follows the page.
 */
export function usePageEditor({
  snapshot,
  latest,
  size,
}: {
  snapshot: EditedPage;
  /** The newest view the page holds, which a landed Save adopts as the new base. */
  latest: MutableRefObject<PageView>;
  size: PageSize;
}) {
  const dirty = useRef(false),
    base = useRef(snapshot.source),
    draftPage = useRef(snapshot.id);
  const [draft, setDraft] = useState(snapshot.source),
    [saving, setSaving] = useState(false);
  const [error, setError] = useState<SaveProblem | null>(null);
  /** The last Save merged the edit into a page that had changed; true until the next edit. */
  const [merged, setMerged] = useState(false);
  /** How the last Copy draft ended. */
  const [copied, setCopied] = useState<'copied' | 'failed' | null>(null);
  const sourceArea = useRef<HTMLTextAreaElement>(null);
  /** Where the caret and scroll were when a merged Save replaced the text, to put them back. */
  const placement = useRef<{ start: number; end: number; top: number } | null>(null);
  // Settles when the Save in flight has landed (true) or failed (false); true when none is.
  const saveSettled = useRef<Promise<boolean> | null>(null);

  /** A replacement of the same page keeps a dirty draft with its frozen base and caret, never
   * saving or retargeting it; a clean draft, or another page, adopts the snapshot. */
  const adoptSnapshot = useCallback((next: EditedPage) => {
    if (!dirty.current || draftPage.current !== next.id) {
      dirty.current = false;
      base.current = next.source;
      setDraft(next.source);
    }
    draftPage.current = next.id;
    setError(null);
  }, []);
  /** A live update of the page's source moves a clean draft; a dirty one stays as typed. */
  const adoptUpdate = useCallback((source: string) => {
    if (!dirty.current) {
      base.current = source;
      setDraft(source);
    }
  }, []);

  async function save() {
    if (!snapshot.binding || saving) return;
    let landed = false;
    let settle!: (landed: boolean) => void;
    saveSettled.current = new Promise<boolean>((resolve) => (settle = resolve));
    setSaving(true);
    setError(null);
    setMerged(false);
    setCopied(null);
    const area = sourceArea.current;
    try {
      const outcome = await snapshot.binding.edit(draft, base.current);
      dirty.current = false;
      base.current = latest.current.source;
      if (outcome.merged && area && document.activeElement === area)
        placement.current = {
          start: area.selectionStart,
          end: area.selectionEnd,
          top: area.scrollTop,
        };
      setDraft(latest.current.source);
      setMerged(outcome.merged);
      void size.refresh();
      landed = true;
    } catch (failure) {
      const problem: SaveProblem = {
        message: saveMessage(failure),
        unconfirmed: failure instanceof SaveOutcomeUnknown,
        ...(failure instanceof SaveConflict ? { conflict: true as const } : {}),
      };
      // A full page names the way out that can help, decided by what the serve says is limiting
      // it now: combining changes frees room only when the changes are the limit.
      if (failure instanceof SaveRefused && failure.code === 'COLAB_CAPACITY') {
        const usage = await size.refresh();
        // A device that cannot ask the serve to combine is told who can.
        if (!snapshot.binding.compact) problem.message = text.pageSize.saveFullReader;
        else if (usage) {
          problem.full = compactOffered(usage) ? 'compact' : 'export';
          problem.message = compactOffered(usage)
            ? text.pageSize.saveFullTail
            : text.pageSize.saveFullBudget;
        }
      }
      setError(problem);
    } finally {
      setSaving(false);
      saveSettled.current = null;
      settle(landed);
    }
  }
  // A merged Save replaces the text with the merged page; the caret and scroll stay where the new
  // text allows, and nothing takes focus.
  useLayoutEffect(() => {
    const area = sourceArea.current,
      kept = placement.current;
    placement.current = null;
    if (!area || !kept || document.activeElement !== area) return;
    area.setSelectionRange(Math.min(kept.start, draft.length), Math.min(kept.end, draft.length));
    area.scrollTop = kept.top;
  }, [draft]);
  /** Copy the unsaved draft from a conflict, since a reload loses it. */
  function copyDraft() {
    void navigator.clipboard.writeText(draft).then(
      () => setCopied('copied'),
      () => setCopied('failed'),
    );
  }
  /** Compact from the save refusal: the draft stays, nothing is saved or resent; the person
   * presses Save again. */
  async function compactFromRefusal() {
    const outcome = await size.compact();
    setError(
      outcome.kind === 'failed'
        ? { message: text.pageSize.failed, unconfirmed: false, full: 'compact' }
        : outcome.kind === 'nothing'
          ? { message: text.pageSize.nothingToCombine, unconfirmed: false, full: 'export' }
          : outcome.kind === 'still-high'
            ? { message: text.pageSize.saveFullBudget, unconfirmed: false, full: 'export' }
            : { message: text.pageSize.compactedSaveAgain, unconfirmed: false, full: 'compacted' },
    );
  }
  /** The person typed in the Source area. */
  function type(value: string) {
    dirty.current = true;
    setMerged(false);
    setDraft(value);
  }

  return {
    draft,
    /** Whether the draft differs from the source it was opened from. */
    changed: draft !== base.current,
    saving,
    error,
    merged,
    copied,
    sourceArea,
    saveSettled,
    adoptSnapshot,
    adoptUpdate,
    save,
    copyDraft,
    compactFromRefusal,
    type,
  };
}
