import { BrowserAction, BrowserDrawer } from '@tmt/browser-ui/react';
import { useEffect, useRef, useState } from 'react';
import './source-editor.css';
import type { EditOutcome } from './link-edit.js';
import { saveMessage } from './save.js';
import { SaveNotice, type SaveProblem } from './save-notice.js';
import { text } from './strings.js';

/** Sends page text as an edit request for an editor link. It resolves once the page answers:
 * `applied` when the page shows the proposed text, `changed` when it shows anything else. A page
 * that has not changed after a while calls `onWaiting`, and the request is still not answered. */
export type ReaderEdit = (
  source: string,
  base: string,
  onWaiting: () => void,
) => Promise<EditOutcome>;

/** The owner's Source drawer, for a link that may edit: the same label, textarea and Save, but a
 * save is a request the owner's Colab applies, so the notices say what the page showed. */
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
    setCopied(null);
    try {
      const answer = await edit(draft, base.current, () =>
        setProblem({ message: text.linkEditWaiting, unconfirmed: true }),
      );
      if (answer.kind === 'applied') {
        dirty.current = false;
        base.current = answer.source;
        setDraft(answer.source);
        setProblem(null);
      } else {
        setProblem({ message: text.linkEditChanged, unconfirmed: true, conflict: true });
      }
    } catch (error) {
      setProblem({ message: saveMessage(error), unconfirmed: false });
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
            {saving && !problem ? text.saving : text.save}
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
        <textarea
          id="source-edit"
          readOnly={saving}
          spellCheck={false}
          value={draft}
          onChange={(event) => {
            dirty.current = true;
            setDraft(event.target.value);
          }}
        />
      </div>
    </BrowserDrawer>
  );
}
