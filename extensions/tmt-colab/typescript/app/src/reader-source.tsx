import { BrowserAction, BrowserDrawer, BrowserNotice } from '@tmt/browser-ui/react';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import './source-editor.css';
import type { EditOutcome } from './link-edit.js';
import { saveMessage } from './save.js';
import { SaveNotice, type SaveProblem } from './save-notice.js';
import { text } from './strings.js';
import { Circle, Diamond, X } from 'lucide-react';

export type EditRequestOutcome = 'sent' | 'refused' | 'unconfirmed' | 'writer-in-use';
/** Provider uncertainty never becomes a refusal or an automatic resend. */
export function EditRequestNotice({
  outcome,
  actions,
}: {
  outcome: EditRequestOutcome;
  actions?: ReactNode;
}) {
  return (
    <div className="source-notice">
      <BrowserNotice
        tone={
          outcome === 'unconfirmed'
            ? 'waiting'
            : outcome === 'refused' || outcome === 'writer-in-use'
              ? 'blocked'
              : 'muted'
        }
        announcement={outcome === 'refused' || outcome === 'writer-in-use' ? 'alert' : 'status'}
        stateLabel={
          outcome === 'writer-in-use'
            ? 'Link in use'
            : outcome === 'unconfirmed'
              ? 'Unconfirmed'
              : outcome === 'refused'
                ? 'Not sent'
                : 'Sent'
        }
        mark={
          outcome === 'unconfirmed' ? (
            <Diamond fill="currentColor" />
          ) : outcome === 'refused' || outcome === 'writer-in-use' ? (
            <X />
          ) : (
            <Circle />
          )
        }
        title={
          outcome === 'writer-in-use'
            ? 'Another browser is already sending edits through this link, so this one can only read. Ask the owner for an editor link of your own.'
            : outcome === 'unconfirmed'
              ? "Couldn't confirm the edit request. It may still reach the owner; check the page before sending it again."
              : outcome === 'refused'
                ? 'Edit request not sent. Ask the owner for a current link.'
                : "Edit request sent. It appears on the page when the owner's Colab applies it."
        }
        body={null}
        actions={actions}
      />
    </div>
  );
}

/** Sends page text as an edit request for an editor link. It resolves once the page answers:
 * `applied` when the page shows the proposed text, `changed` when it shows anything else. A page
 * that has not changed after a while calls `onWaiting`, and the request is still not answered. */
export type ReaderEdit = (
  source: string,
  base: string,
  onWaiting: () => void,
) => Promise<EditOutcome | { request: EditRequestOutcome }>;

/** The owner's Source drawer, for a link that may edit: the same label, textarea and Save, but a
 * save is a request the owner's Colab applies, so the notices say what the page showed. */
export function ReaderSource({
  source,
  edit,
  open,
  onDismiss,
  writerBlocked,
  onWriterInUse,
}: {
  /** The text of the page as the reader last admitted it. */
  source: string;
  edit: ReaderEdit;
  open: boolean;
  onDismiss: () => void;
  writerBlocked: boolean;
  onWriterInUse(): void;
}) {
  const dirty = useRef(false),
    base = useRef(source);
  const [draft, setDraft] = useState(source),
    [saving, setSaving] = useState(false),
    [problem, setProblem] = useState<SaveProblem | null>(null),
    [copied, setCopied] = useState<'copied' | 'failed' | null>(null),
    [request, setRequest] = useState<EditRequestOutcome | null>(null),
    [sentDraft, setSentDraft] = useState<string | null>(null);
  const latestDraft = useRef(draft);
  latestDraft.current = draft;
  // The page moved elsewhere while nothing is being edited: show it, and edit from it.
  useEffect(() => {
    if (dirty.current && source !== latestDraft.current) return;
    dirty.current = false;
    base.current = source;
    setDraft(source);
  }, [source]);
  async function save() {
    if (saving || writerBlocked) return;
    setSaving(true);
    setProblem(null);
    setCopied(null);
    setRequest(null);
    try {
      const answer = await edit(draft, base.current, () =>
        setProblem({ message: text.linkEditWaiting, unconfirmed: true }),
      );
      if ('request' in answer) {
        setRequest(answer.request);
        if (answer.request === 'sent') setSentDraft(draft);
        if (answer.request === 'writer-in-use') onWriterInUse();
        return;
      }
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
  const copyDraft = (
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
  );
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
          <button
            disabled={saving || writerBlocked || draft === base.current || draft === sentDraft}
            onClick={() => void save()}
          >
            {saving && !problem ? text.saving : text.save}
          </button>
        </span>
        {request && (
          <EditRequestNotice
            outcome={request}
            actions={request === 'writer-in-use' ? copyDraft : undefined}
          />
        )}
        {problem && (
          <SaveNotice problem={problem} actions={problem.conflict ? copyDraft : undefined} />
        )}
        <textarea
          id="source-edit"
          readOnly={saving}
          spellCheck={false}
          value={draft}
          onChange={(event) => {
            dirty.current = true;
            if (!writerBlocked) setRequest(null);
            setDraft(event.target.value);
          }}
        />
      </div>
    </BrowserDrawer>
  );
}
