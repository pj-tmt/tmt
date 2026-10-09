import { BrowserAction } from '@tmt/browser-ui/react';
import { useEffect, useRef, useState } from 'react';
import type { attachment } from '@tmt/colab-client';
import { GENERIC_MEDIA_TYPE, RASTER_TYPES, previewType } from './attachment-file.js';
import type { AttachmentBinding } from './attachment-service.js';
import { BlobDownloads } from './export.js';
import { text } from './strings.js';
import type { CommentView } from './thread-records.js';
import './attachment-tray.css';

const dataUrl = (blob: Blob) =>
  new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });

/** Bytes open only on an explicit click, each through the admitted read of this exact
 * message revision. A preview is a bounded raster shown from a `data:` URL; every other
 * file is only ever an octet-stream download. Nothing here renders on its own. */
export function MessageAttachments({
  comment,
  binding,
}: {
  comment: CommentView;
  binding?: AttachmentBinding;
}) {
  const downloads = useRef<BlobDownloads | null>(null);
  useEffect(() => {
    const owner = new BlobDownloads();
    downloads.current = owner;
    return () => {
      owner.close();
      downloads.current = null;
    };
  }, []);
  const [busy, setBusy] = useState<string>();
  const [failed, setFailed] = useState<Set<string>>(new Set());
  const [shown, setShown] = useState<Record<string, string>>({});
  if (!comment.attachments?.length) return null;
  const run = async (
    d: attachment.AttachmentDescriptor,
    use: (bytes: Uint8Array) => Promise<void>,
  ) => {
    if (busy || !binding) return;
    setBusy(d.attachmentId);
    setFailed((previous) => {
      const next = new Set(previous);
      next.delete(d.attachmentId);
      return next;
    });
    let bytes: Uint8Array | undefined;
    try {
      bytes = await binding.open(
        { writer: comment.ref.writer, messageId: comment.messageId, revision: comment.revision },
        d,
      );
      await use(bytes);
    } catch {
      setFailed((previous) => new Set(previous).add(d.attachmentId));
    } finally {
      bytes?.fill(0);
      setBusy(undefined);
    }
  };
  return (
    <ul
      className="message-attachments"
      aria-label={text.messageAttachmentsLabel}
      data-testid="message-attachments"
    >
      {comment.attachments.map((d) => {
        const raster = (RASTER_TYPES as readonly string[]).includes(d.mediaType),
          image = shown[d.attachmentId],
          opening = busy === d.attachmentId;
        return (
          <li key={d.attachmentId} className="message-attachment" data-testid="message-attachment">
            <span className="attachment-name">{d.filename}</span>
            <span className="attachment-size">{text.attachmentSize(Number(d.plaintextBytes))}</span>
            {failed.has(d.attachmentId) && (
              <span className="attachment-state" role="alert">
                {text.attachmentUnavailable}
              </span>
            )}
            <span className="attachment-actions">
              {raster &&
                (image ? (
                  <BrowserAction
                    type="button"
                    variant="text"
                    label={text.attachmentHidePreview}
                    onActivate={(event) => {
                      if (event.isTrusted)
                        setShown(({ [d.attachmentId]: _hidden, ...rest }) => rest);
                    }}
                  />
                ) : (
                  <BrowserAction
                    type="button"
                    variant="text"
                    label={opening ? text.attachmentOpening : text.attachmentPreview}
                    busy={opening}
                    disabled={!!busy}
                    onActivate={(event) => {
                      if (event.isTrusted)
                        void run(d, async (bytes) => {
                          const type = previewType(d.mediaType, bytes);
                          if (!type) throw new Error('Not a bounded raster');
                          const url = await dataUrl(new Blob([new Uint8Array(bytes)], { type }));
                          setShown((previous) => ({ ...previous, [d.attachmentId]: url }));
                        });
                    }}
                  />
                ))}
              <BrowserAction
                type="button"
                variant="text"
                label={!raster && opening ? text.attachmentOpening : text.attachmentDownload}
                busy={!raster && opening}
                disabled={!!busy}
                onActivate={(event) => {
                  if (event.isTrusted)
                    void run(d, async (bytes) =>
                      downloads.current?.request(
                        new Blob([new Uint8Array(bytes)], { type: GENERIC_MEDIA_TYPE }),
                        d.filename,
                      ),
                    );
                }}
              />
            </span>
            {image && (
              <div className="attachment-preview">
                <img src={image} alt={d.filename} />
              </div>
            )}
          </li>
        );
      })}
    </ul>
  );
}
