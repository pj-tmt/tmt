import { BrowserAction, BrowserDrawer } from '@tmt/browser-ui/react';
import { useEffect, useRef, useState } from 'react';
import './source-editor.css';
import { SaveConflict, SaveOutcomeUnknown, SaveRefused, saveMessage } from './save.js';
import { SaveNotice, type SaveProblem } from './save-notice.js';
import { text } from './strings.js';

/** Saves page text for an editor link: the serve prepares and signs it, so the result is the
 * admitted page, whose text comes back. */
export type ReaderEdit = (
  source: string,
  base: string,
) => Promise<{ merged: boolean; source: string }>;

/** The owner's Source drawer, for a link that may edit: the same label, textarea, Save and notices.
 * Compaction is an editor's to offer, so a full page says who can free room. */
export function ReaderSource({
  source,
  edit,
  open,
  onDismiss,
}: {
  /** The text of the page as the reader last admitted it. */
  source: string;
  edit: ReaderEdit;
  open: boolean;
  onDismiss: () => void;
}) {
  const dirty = useRef(false),
    base = useRef(source);
  const [draft, setDraft] = useState(source),
    [saving, setSaving] = useState(false),
    [problem, setProblem] = useState<SaveProblem | null>(null),
    [merged, setMerged] = useState(false),
    [copied, setCopied] = useState<'copied' | 'failed' | null>(null);
  // The page moved elsewhere while nothing is being edited: show it, and edit from it.
  useEffect(() => {
    if (dirty.current) return;
    base.current = source;
    setDraft(source);
  }, [source]);
  async function save() {
    if (saving) return;
    setSaving(true);
    setProblem(null);
    setMerged(false);
    setCopied(null);
    try {
      const saved = await edit(draft, base.current);
      dirty.current = false;
      base.current = saved.source;
      setDraft(saved.source);
      setMerged(saved.merged);
    } catch (error) {
      setProblem({
        message:
          error instanceof SaveRefused && error.code === 'COLAB_CAPACITY'
            ? text.pageSize.saveFullReader
            : saveMessage(error),
        unconfirmed: error instanceof SaveOutcomeUnknown,
        ...(error instanceof SaveConflict ? { conflict: true as const } : {}),
      });
    } finally {
      setSaving(false);
    }
  }
  return (
    <BrowserDrawer
      focusFallback={() => document.querySelector<HTMLButtonElement>('.tmt-ui-header button')}
      open={open}
      title={text.source}
      panel="source"
      width="wide"
      onDismiss={onDismiss}
    >
      <div className="source">
        <span>
          <label htmlFor="source-edit">{text.source}</label>
          <button disabled={saving || draft === base.current} onClick={() => void save()}>
            {saving ? text.saving : text.save}
          </button>
        </span>
        {problem && (
          <SaveNotice
            problem={problem}
            actions={
              problem.conflict ? (
                <>
                  <BrowserAction
                    type="button"
                    label={text.saveCopyDraft}
                    variant="text"
                    onActivate={(event) => {
                      if (event.isTrusted)
                        void navigator.clipboard.writeText(draft).then(
                          () => setCopied('copied'),
                          () => setCopied('failed'),
                        );
                    }}
                  />
                  {copied && (
                    <span role="status">
                      {copied === 'copied' ? text.saveDraftCopied : text.saveDraftNotCopied}
                    </span>
                  )}
                </>
              ) : undefined
            }
          />
        )}
        {merged && !problem && (
          <div className="source-notice">
            <p role="status">{text.saveMerged}</p>
          </div>
        )}
        <textarea
          id="source-edit"
          readOnly={saving}
          spellCheck={false}
          value={draft}
          onChange={(event) => {
            dirty.current = true;
            setMerged(false);
            setDraft(event.target.value);
          }}
        />
      </div>
    </BrowserDrawer>
  );
}
