import {
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type MutableRefObject,
} from 'react';
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
}

/** What the Source editor shows: the text being edited, whether it differs from the source it was
 * opened from, and how many snapshots the draft has adopted (a new one clears the Save notice). */
export interface SourceDraftState {
  readonly text: string;
  readonly changed: boolean;
  readonly opened: number;
}

/**
 * The Source draft of an open page: the text being edited and the source it was opened from
 * (`base`, frozen while the draft is dirty). The page route owns one and feeds it what arrives: a
 * replaced snapshot (`adoptSnapshot`) and each live update (`adoptUpdate`); a dirty draft keeps its
 * frozen base, never saved or retargeted, and a clean one follows the page. Only the Source drawer
 * subscribes, so a keystroke renders the editor and not the page around it.
 */
export class SourceDraft {
  #dirty = false;
  #base: string;
  #page: string;
  #state: SourceDraftState;
  #listeners = new Set<() => void>();
  /** Settles when the Save in flight has landed (true) or failed (false); null when none is. */
  saveSettled: Promise<boolean> | null = null;

  constructor(page: EditedPage) {
    this.#base = page.source;
    this.#page = page.id;
    this.#state = { text: page.source, changed: false, opened: 0 };
  }
  subscribe = (listener: () => void) => {
    this.#listeners.add(listener);
    return () => void this.#listeners.delete(listener);
  };
  read = () => this.#state;
  /** The source the draft was opened from, which a Save sends as its base. */
  get base() {
    return this.#base;
  }
  /** A replacement of the same page keeps a dirty draft with its frozen base, never saving or
   * retargeting it; a clean draft, or another page, adopts the snapshot. */
  adoptSnapshot(next: EditedPage) {
    const keep = this.#dirty && this.#page === next.id;
    if (!keep) {
      this.#dirty = false;
      this.#base = next.source;
    }
    this.#page = next.id;
    this.#set(keep ? this.#state.text : next.source, this.#state.opened + 1);
  }
  /** A live update of the page's source moves a clean draft; a dirty one stays as typed. */
  adoptUpdate(source: string) {
    if (this.#dirty) return;
    this.#base = source;
    this.#set(source);
  }
  /** The person typed in the Source area. */
  type(value: string) {
    this.#dirty = true;
    this.#set(value);
  }
  /** A Save landed: the page's newest source is the new base and the text. */
  landed(source: string) {
    this.#dirty = false;
    this.#base = source;
    this.#set(source);
  }
  #set(value: string, opened = this.#state.opened) {
    const changed = value !== this.#base;
    const state = this.#state;
    if (state.text === value && state.changed === changed && state.opened === opened) return;
    this.#state = { text: value, changed, opened };
    for (const listener of this.#listeners) listener();
  }
}

/**
 * The Source editor's view of a page's draft and its Save: the draft's text, how the last Save
 * ended, and where the caret was when a merged Save replaced the text. A Save reads the draft as
 * it stands when it starts.
 */
export function usePageEditor({
  draft,
  binding,
  latest,
  size,
}: {
  draft: SourceDraft;
  binding: PageBinding | undefined;
  /** The newest view the page holds, which a landed Save adopts as the new base. */
  latest: MutableRefObject<PageView>;
  size: PageSize;
}) {
  const current = useSyncExternalStore(draft.subscribe, draft.read);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<SaveProblem | null>(null);
  // A newly adopted snapshot clears the last Save's notice.
  const [cleared, setCleared] = useState(current.opened);
  if (cleared !== current.opened) {
    setCleared(current.opened);
    setError(null);
  }
  /** The last Save merged the edit into a page that had changed; true until the next edit. */
  const [merged, setMerged] = useState(false);
  /** How the last Copy draft ended. */
  const [copied, setCopied] = useState<'copied' | 'failed' | null>(null);
  const sourceArea = useRef<HTMLTextAreaElement>(null);
  /** Where the caret and scroll were when a merged Save replaced the text, to put them back. */
  const placement = useRef<{ start: number; end: number; top: number } | null>(null);

  async function save() {
    if (!binding || saving) return;
    let landed = false;
    let settle!: (landed: boolean) => void;
    draft.saveSettled = new Promise<boolean>((resolve) => (settle = resolve));
    setSaving(true);
    setError(null);
    setMerged(false);
    setCopied(null);
    const area = sourceArea.current;
    try {
      const outcome = await binding.edit(draft.read().text, draft.base);
      if (outcome.merged && area && document.activeElement === area)
        placement.current = {
          start: area.selectionStart,
          end: area.selectionEnd,
          top: area.scrollTop,
        };
      draft.landed(latest.current.source);
      setMerged(outcome.merged);
      void size.refresh();
      landed = true;
    } catch (failure) {
      const problem: SaveProblem = {
        message:
          failure instanceof SaveRefused && failure.findings?.length && !binding.canMetadata?.()
            ? text.metadata.ownerOnly
            : saveMessage(failure),
        unconfirmed: failure instanceof SaveOutcomeUnknown,
        ...(failure instanceof SaveConflict ? { conflict: true as const } : {}),
      };
      // A full page names the way out that can help, decided by what the serve says is limiting
      // it now: combining changes frees room only when the changes are the limit.
      if (failure instanceof SaveRefused && failure.code === 'COLAB_CAPACITY') {
        const usage = await size.refresh();
        // A device that cannot ask the serve to combine is told who can.
        if (!binding.compact) problem.message = text.pageSize.saveFullReader;
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
      draft.saveSettled = null;
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
    const length = current.text.length;
    area.setSelectionRange(Math.min(kept.start, length), Math.min(kept.end, length));
    area.scrollTop = kept.top;
  }, [current.text]);
  /** Copy the unsaved draft from a conflict, since a reload loses it. */
  function copyDraft() {
    void navigator.clipboard.writeText(draft.read().text).then(
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
    setMerged(false);
    draft.type(value);
  }

  return {
    draft: current.text,
    /** Whether the draft differs from the source it was opened from. */
    changed: current.changed,
    saving,
    error,
    merged,
    copied,
    sourceArea,
    save,
    copyDraft,
    compactFromRefusal,
    type,
  };
}
