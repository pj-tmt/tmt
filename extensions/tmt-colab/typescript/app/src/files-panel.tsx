import { BrowserAction, BrowserList, BrowserListRow } from '@tmt/browser-ui/react';
import { Fragment, useEffect, useState, useSyncExternalStore } from 'react';
import type { attachment } from '@tmt/colab-client';
import { AttachmentDraft } from './attachment-draft.js';
import { isRaster, useAttachmentOpener } from './attachment-opener.js';
import { AttachmentStaleError, type AttachmentBinding } from './attachment-service.js';
import { AttachButton, AttachmentChips } from './attachment-tray.js';
import type { DocumentFiles } from './document-files.js';
import { text } from './strings.js';
import './attachment-tray.css';

/** The most files one page document holds (mirrors the native bound). */
const PAGE_FILES = 128;
const DOCUMENT = { kind: 'document' } as const;

/** The page's files as list rows. Bytes open only on an explicit click, through the admitted
 * read of the document's current content; `remove` is given to writers only. */
export function FilesList({
  descriptors,
  binding,
  remove,
  disabled = false,
  epoch,
}: {
  descriptors: readonly attachment.AttachmentDescriptor[];
  binding?: AttachmentBinding;
  /** The page's current epoch; a file sealed under an earlier one says it is being secured. */
  epoch?: string;
  remove?(attachmentId: string): Promise<void>;
  disabled?: boolean;
}) {
  const opener = useAttachmentOpener(binding),
    { busy, failed, shown } = opener;
  const [removing, setRemoving] = useState<string>();
  const [removeFailed, setRemoveFailed] = useState<Set<string>>(new Set());
  if (!descriptors.length) return null;
  const drop = async (id: string) => {
    if (!remove || removing) return;
    setRemoving(id);
    setRemoveFailed((previous) => {
      const next = new Set(previous);
      next.delete(id);
      return next;
    });
    try {
      await remove(id);
    } catch {
      setRemoveFailed((previous) => new Set(previous).add(id));
    } finally {
      setRemoving(undefined);
    }
  };
  return (
    <BrowserList label={text.filesListLabel} className="files-list">
      {descriptors.map((d) => {
        const raster = isRaster(d),
          image = shown[d.attachmentId],
          opening = busy === d.attachmentId,
          idle = !busy && !removing;
        return (
          <Fragment key={d.attachmentId}>
            <BrowserListRow
              data-testid="file-row"
              data-previewed={image ? '' : undefined}
              title={<span className="attachment-name">{d.filename}</span>}
              metadata={text.attachmentSize(Number(d.plaintextBytes))}
              state={
                failed.has(d.attachmentId) ? (
                  <span role="alert">{text.attachmentReason[failed.get(d.attachmentId)!]}</span>
                ) : removeFailed.has(d.attachmentId) ? (
                  <span role="alert">{text.filesRemoveFailed}</span>
                ) : removing === d.attachmentId ? (
                  text.filesRemoving
                ) : epoch !== undefined && d.epoch !== epoch ? (
                  text.filesResealing
                ) : null
              }
              actions={
                <>
                  {raster &&
                    (image ? (
                      <BrowserAction
                        type="button"
                        variant="text"
                        label={text.attachmentHidePreview}
                        onActivate={(event) => {
                          if (event.isTrusted) opener.hide(d.attachmentId);
                        }}
                      />
                    ) : (
                      <BrowserAction
                        type="button"
                        variant="text"
                        label={opening ? text.attachmentOpening : text.attachmentPreview}
                        busy={opening}
                        disabled={!idle || !binding}
                        onActivate={(event) => {
                          if (event.isTrusted) void opener.preview(DOCUMENT, d);
                        }}
                      />
                    ))}
                  <BrowserAction
                    type="button"
                    variant="text"
                    label={!raster && opening ? text.attachmentOpening : text.attachmentDownload}
                    busy={!raster && opening}
                    disabled={!idle || !binding}
                    onActivate={(event) => {
                      if (event.isTrusted) void opener.download(DOCUMENT, d);
                    }}
                  />
                  {remove && (
                    <BrowserAction
                      type="button"
                      variant="text"
                      label={text.attachRemove}
                      disabled={!idle || disabled}
                      onActivate={(event) => {
                        if (event.isTrusted) void drop(d.attachmentId);
                      }}
                    />
                  )}
                </>
              }
            />
            {image && (
              <li className="file-preview" data-testid="file-preview">
                <img src={image} alt={d.filename} />
              </li>
            )}
          </Fragment>
        );
      })}
    </BrowserList>
  );
}

/** The writer's Files panel: the page's files, then the attach row. Choosing files only adds
 * local chips; "Add to page" uploads them, proves them, and saves the references. */
export function FilesPanel({
  descriptors,
  files,
  disabled,
}: {
  descriptors: readonly attachment.AttachmentDescriptor[];
  files: DocumentFiles;
  disabled: boolean;
}) {
  const [draft] = useState(
    () =>
      new AttachmentDraft(
        () => files.attachments,
        () => files.target(),
      ),
  );
  useEffect(() => () => draft.dispose(), [draft]);
  const staged = useSyncExternalStore(draft.subscribe, draft.getSnapshot);
  const [adding, setAdding] = useState(false);
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
  const attachable = !disabled && !adding;
  return (
    <div className="files-panel" data-testid="files-panel">
      {descriptors.length ? (
        <FilesList
          descriptors={descriptors}
          binding={files.attachments}
          remove={(id) => files.remove([id])}
          disabled={disabled || adding}
          epoch={files.epoch}
        />
      ) : (
        !staged.chips.length && <p className="files-empty">{text.filesEmpty}</p>
      )}
      <AttachmentChips draft={draft} disabled={!attachable} page />
      <div className="annotation-status-row">
        <AttachButton draft={draft} disabled={!attachable} labeled />
        <p role="status" className="annotation-hint">
          {error}
        </p>
        {staged.chips.length > 0 && (
          <BrowserAction
            type="button"
            variant="text"
            label={adding ? text.filesAdding : text.filesAdd}
            busy={adding}
            disabled={!attachable || draft.uploading}
            onActivate={(event) => {
              if (event.isTrusted) void add();
            }}
          />
        )}
      </div>
    </div>
  );
}
