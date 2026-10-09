import { BrowserAction } from '@tmt/browser-ui/react';
import { isRaster, useAttachmentOpener } from './attachment-opener.js';
import type { AttachmentBinding } from './attachment-service.js';
import { text } from './strings.js';
import type { CommentView } from './thread-records.js';
import './attachment-tray.css';

/** Bytes open only on an explicit click, each through the admitted read of this exact
 * message revision. Nothing here renders on its own. */
export function MessageAttachments({
  comment,
  binding,
}: {
  comment: CommentView;
  binding?: AttachmentBinding;
}) {
  const opener = useAttachmentOpener(binding),
    { busy, failed, shown } = opener;
  if (!comment.attachments?.length) return null;
  const reference = {
    kind: 'message',
    writer: comment.ref.writer,
    messageId: comment.messageId,
    revision: comment.revision,
  } as const;
  return (
    <ul
      className="message-attachments"
      aria-label={text.messageAttachmentsLabel}
      data-testid="message-attachments"
    >
      {comment.attachments.map((d) => {
        const raster = isRaster(d),
          image = shown[d.attachmentId],
          opening = busy === d.attachmentId;
        return (
          <li key={d.attachmentId} className="message-attachment" data-testid="message-attachment">
            <span className="attachment-name">{d.filename}</span>
            <span className="attachment-size">{text.attachmentSize(Number(d.plaintextBytes))}</span>
            {failed.has(d.attachmentId) && (
              <span className="attachment-state" role="alert">
                {text.attachmentReason[failed.get(d.attachmentId)!]}
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
                      if (event.isTrusted) opener.hide(d.attachmentId);
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
                      if (event.isTrusted) void opener.preview(reference, d);
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
                  if (event.isTrusted) void opener.download(reference, d);
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
