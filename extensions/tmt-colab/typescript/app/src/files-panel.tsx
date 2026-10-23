import {
  BrowserAction,
  BrowserDialog,
  BrowserDrawer,
  BrowserList,
  BrowserListRow,
  BusyMark,
  type BrowserDrawerProps,
} from '@tmt/browser-ui/react';
import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react';
import { Download, Eye, FileText, Image as ImageIcon, Trash2, Upload } from 'lucide-react';
import type { attachment } from '@tmt/colab-client';
import { AttachmentDraft } from './attachment-draft.js';
import { isRaster, useAttachmentOpener } from './attachment-opener.js';
import { AttachmentStaleError, type AttachmentBinding } from './attachment-service.js';
import { AttachmentChips } from './attachment-tray.js';
import type { DocumentFiles } from './document-files.js';
import { text } from './strings.js';
import './attachment-tray.css';

/** The most files one page document holds (mirrors the native bound). */
const PAGE_FILES = 128;
const DOCUMENT = { kind: 'document' } as const;
const settled = () => Promise.resolve(true);
type FileOverlay = {
  kind: 'preview' | 'remove';
  lifetime?: object;
  id: string;
  filename: string;
  trigger: HTMLElement;
};

/** The Files surface owns its one overlay and focus return. Drafts and admitted byte reads stay
 * mounted when a preview or confirmation replaces the drawer; commands remain with their owners. */
export function FilesSurface({
  open,
  onDismiss,
  focusFallback,
  notice,
  descriptors,
  binding,
  files,
  disabled = false,
  readOnly = false,
  afterSave = settled,
}: Pick<BrowserDrawerProps, 'open' | 'onDismiss' | 'focusFallback' | 'notice'> & {
  descriptors: readonly attachment.AttachmentDescriptor[];
  binding?: AttachmentBinding;
  files?: DocumentFiles;
  disabled?: boolean;
  readOnly?: boolean;
  afterSave?(): Promise<boolean>;
}) {
  const opener = useAttachmentOpener(files?.attachments ?? binding, DOCUMENT, descriptors);
  const [overlay, setOverlay] = useState<FileOverlay>();
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<string>();
  const removingNow = useRef(false);
  const [removeFailed, setRemoveFailed] = useState<Set<string>>(new Set());
  const returnTo = useRef<HTMLElement | null>(null);
  const current = useRef({ files, disabled, readOnly, descriptors, afterSave });
  current.current = { files, disabled, readOnly, descriptors, afterSave };
  const selected = overlay && descriptors.find((d) => d.attachmentId === overlay.id);
  const working = !!opener.busy || !!removing;
  const closeOverlay = () => {
    if (removingNow.current) return;
    if (overlay?.kind === 'preview') opener.hide(overlay.id);
    returnTo.current = overlay?.trigger ?? null;
    setOverlay(undefined);
  };
  useEffect(() => {
    if (!open && overlay && !removingNow.current) {
      if (overlay.kind === 'preview') opener.hide(overlay.id);
      returnTo.current = null;
      setOverlay(undefined);
    }
  }, [open, overlay, opener]);
  useLayoutEffect(() => {
    if (!open || overlay || !returnTo.current) return;
    const trigger = returnTo.current;
    returnTo.current = null;
    if (trigger.isConnected && trigger.getClientRects().length)
      trigger.focus({ preventScroll: true });
    else focusFallback?.()?.focus({ preventScroll: true });
  }, [open, overlay, focusFallback]);
  const remove = async () => {
    if (!overlay || overlay.kind !== 'remove' || removingNow.current || working) return;
    const id = overlay.id;
    const admitted = () => {
      const now = current.current;
      return (
        now.files &&
        !now.disabled &&
        !now.readOnly &&
        now.descriptors.some((d) => d.attachmentId === id)
      );
    };
    if (!admitted()) return;
    removingNow.current = true;
    setRemoving(id);
    setRemoveFailed((previous) => {
      const next = new Set(previous);
      next.delete(id);
      return next;
    });
    try {
      if (await current.current.afterSave()) {
        if (!admitted()) throw new Error('File removal is no longer admitted');
        await current.current.files!.remove([id]);
      }
    } catch {
      setRemoveFailed((previous) => new Set(previous).add(id));
    } finally {
      removingNow.current = false;
      setRemoving(undefined);
      returnTo.current = overlay.trigger;
      setOverlay(undefined);
    }
  };
  return (
    <>
      <BrowserDrawer
        open={open && !overlay}
        title={text.files}
        panel="files"
        onDismiss={onDismiss}
        focusFallback={focusFallback}
        notice={notice}
      >
        <div className="files-panel" data-testid="files-panel">
          <h3>{text.filesCount(descriptors.length)}</h3>
          {opener.unlistedFailure && <p role="alert">{text.attachmentReason.changed}</p>}
          {descriptors.length > 0 && (
            <BrowserList label={text.filesListLabel} className="files-list">
              {descriptors.map((d) => {
                const raster = isRaster(d),
                  opening = opener.busy === d.attachmentId;
                const Icon = raster ? ImageIcon : FileText;
                const mark = (active: boolean) => (active ? <BusyMark /> : <span />);
                return (
                  <BrowserListRow
                    key={d.attachmentId}
                    data-testid="file-row"
                    data-attachment-id={d.attachmentId}
                    leading={
                      <span className="files-type-mark">
                        <Icon aria-hidden />
                      </span>
                    }
                    title={<span className="attachment-name">{d.filename}</span>}
                    metadata={text.attachmentSize(Number(d.plaintextBytes))}
                    state={
                      opener.failed.has(d.attachmentId) ? (
                        <span role="alert">
                          {text.attachmentReason[opener.failed.get(d.attachmentId)!]}
                        </span>
                      ) : removeFailed.has(d.attachmentId) ? (
                        <span role="alert">{text.filesRemoveFailed}</span>
                      ) : files && d.epoch !== files.epoch ? (
                        text.filesResealing
                      ) : null
                    }
                    actions={
                      <>
                        {!readOnly && raster && (
                          <BrowserAction
                            type="button"
                            variant="secondary"
                            label={text.attachmentPreview}
                            leadingIcon={<Eye />}
                            busy={working}
                            busyMark={mark(opening)}
                            disabled={!(files?.attachments ?? binding)}
                            onActivate={(event) => {
                              if (event.isTrusted) {
                                setOverlay({
                                  kind: 'preview',
                                  lifetime: opener.currentLifetime(),
                                  id: d.attachmentId,
                                  filename: d.filename,
                                  trigger: event.currentTarget,
                                });
                                void opener.preview(d);
                              }
                            }}
                          />
                        )}
                        <BrowserAction
                          type="button"
                          variant="secondary"
                          label={text.attachmentDownload}
                          leadingIcon={<Download />}
                          busy={working}
                          busyMark={mark(opening && !raster)}
                          disabled={!(files?.attachments ?? binding)}
                          onActivate={(event) => {
                            if (event.isTrusted) void opener.download(d);
                          }}
                        />
                        {files && !readOnly && (
                          <BrowserAction
                            type="button"
                            variant="ghost"
                            label={text.attachRemove}
                            leadingIcon={<Trash2 />}
                            busy={working}
                            busyMark={mark(removing === d.attachmentId)}
                            disabled={disabled || adding}
                            onActivate={(event) => {
                              if (event.isTrusted)
                                setOverlay({
                                  kind: 'remove',
                                  id: d.attachmentId,
                                  filename: d.filename,
                                  trigger: event.currentTarget,
                                });
                            }}
                          />
                        )}
                      </>
                    }
                  />
                );
              })}
            </BrowserList>
          )}
          {files ? (
            <FilesUpload
              descriptors={descriptors}
              files={files}
              disabled={disabled}
              afterSave={afterSave}
              readOnly={readOnly}
              adding={adding}
              setAdding={setAdding}
            />
          ) : (
            !descriptors.length && <p className="files-empty">{text.filesEmpty}</p>
          )}
        </div>
      </BrowserDrawer>
      <BrowserDialog
        open={open && overlay?.kind === 'preview'}
        title={overlay?.filename ?? text.files}
        restoreFocus={false}
        onDismiss={closeOverlay}
        className="files-preview-dialog"
        actions={
          <BrowserAction
            type="button"
            variant="secondary"
            label={text.filesClose}
            onActivate={closeOverlay}
          />
        }
      >
        {!selected || (overlay?.kind === 'preview' && overlay.lifetime !== opener.lifetime) ? (
          <p role="alert">{text.attachmentReason.changed}</p>
        ) : opener.failed.has(selected.attachmentId) ? (
          <p role="alert">{text.attachmentReason[opener.failed.get(selected.attachmentId)!]}</p>
        ) : opener.shown[selected.attachmentId] ? (
          <div data-testid="file-preview">
            <img src={opener.shown[selected.attachmentId]} alt={selected.filename} />
          </div>
        ) : (
          <div role="status" aria-label={text.attachmentOpening}>
            <BusyMark />
          </div>
        )}
      </BrowserDialog>
      <BrowserDialog
        open={open && overlay?.kind === 'remove'}
        title={text.filesRemoveTitle(overlay?.filename ?? '')}
        restoreFocus={false}
        onDismiss={closeOverlay}
        actions={
          <>
            <BrowserAction
              type="button"
              variant="secondary"
              label={text.filesCancel}
              busy={!!removing}
              onActivate={closeOverlay}
            />
            <BrowserAction
              type="button"
              variant="destructive"
              label={text.filesRemoveConfirm}
              busy={!!removing}
              busyMark={<BusyMark />}
              disabled={!selected || disabled || readOnly || !files}
              onActivate={(event) => {
                if (event.isTrusted) void remove();
              }}
            />
          </>
        }
      >
        <p>{text.filesRemoveHint}</p>
        {!selected && <p role="alert">{text.attachmentReason.changed}</p>}
      </BrowserDialog>
    </>
  );
}

/** The writer's retained staging owner. Choosing files only adds local chips; Add to page
 * uploads them, proves them, and saves the references. The surface owns the file rows. */
function FilesUpload({
  descriptors,
  files,
  disabled,
  afterSave,
  readOnly = false,
  adding,
  setAdding,
}: {
  adding: boolean;
  setAdding(value: boolean): void;
  descriptors: readonly attachment.AttachmentDescriptor[];
  files: DocumentFiles;
  disabled: boolean;
  /** Settles once a Source save in flight has landed: true if it did, false if it failed. */
  afterSave(): Promise<boolean>;
  readOnly?: boolean;
}) {
  // A same-page reopen may replace the files owner; the draft keeps its picked bytes and reads
  // the current owner at each upload.
  const owner = useRef(files);
  useEffect(() => {
    owner.current = files;
  }, [files]);
  const [draft] = useState(
    () =>
      new AttachmentDraft(
        () => owner.current.attachments,
        () => owner.current.target(),
      ),
  );
  useEffect(() => () => draft.dispose(), [draft]);
  const staged = useSyncExternalStore(draft.subscribe, draft.getSnapshot);
  const [error, setError] = useState<string>();
  const add = async () => {
    if (adding || disabled) return;
    setError(undefined);
    if (descriptors.length + staged.chips.length > PAGE_FILES) {
      setError(text.filesFull);
      return;
    }
    setAdding(true);
    try {
      // A save in flight lands first; if it fails, nothing is added and the chips stay staged.
      if (!(await afterSave())) return;
      const prepared = await draft.prepare();
      if (!prepared.ok) {
        setError(text.filesBlocked[prepared.why]);
        return;
      }
      if (prepared.attach) await files.add(prepared.attach.stored);
      draft.committed();
    } catch (failure) {
      if (failure instanceof AttachmentStaleError) {
        draft.stale(failure.attachmentIds);
        setError(text.filesBlocked.stale);
      } else setError(text.filesBlocked.save);
    } finally {
      setAdding(false);
    }
  };
  const attachable = !readOnly && !disabled && !adding;
  return (
    <section className="files-add">
      {!readOnly && (
        <>
          <h3>{text.filesChooseTitle}</h3>
          <p>{text.filesChooseHint}</p>
        </>
      )}
      {!descriptors.length && !staged.chips.length && (
        <p className="files-empty">{text.filesEmpty}</p>
      )}
      {!readOnly && (
        <>
          <div className="annotation-status-row">
            <input
              type="file"
              multiple
              aria-label={text.filesChoose}
              disabled={!attachable}
              onChange={(event) => {
                const selected = [...(event.currentTarget.files ?? [])];
                event.currentTarget.value = '';
                if (selected.length) void draft.add(selected);
              }}
            />
            <AttachmentChips draft={draft} disabled={!attachable} page />
            <p role="status" className="annotation-hint">
              {error}
            </p>
            {staged.chips.length > 0 && (
              <BrowserAction
                type="button"
                variant="primary"
                leadingIcon={<Upload />}
                label={text.filesAdd}
                busy={adding}
                busyMark={<BusyMark />}
                disabled={disabled || (!adding && draft.uploading)}
                onActivate={(event) => {
                  if (event.isTrusted) void add();
                }}
              />
            )}
          </div>
          <p className="files-upload-note">{text.filesChooseNote}</p>
        </>
      )}
    </section>
  );
}
