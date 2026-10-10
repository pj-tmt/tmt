import { BrowserIconAction } from '@tmt/browser-ui/react';
import { Download } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import type { attachment } from '@tmt/colab-client';
import { isRaster, useAttachmentOpener } from './attachment-opener.js';
import { AttachmentPreview } from './attachment-preview.js';
import type { AttachmentBinding } from './attachment-service.js';
import { text } from './strings.js';
import type { CommentView } from './thread-records.js';
import './attachment-tray.css';

/** Observer delivery may trail a scroll. Recheck actual clipped geometry at read admission. */
function visibleTile(node: HTMLElement | null): boolean {
  if (!node?.isConnected || getComputedStyle(node).visibility !== 'visible') return false;
  const rect = node.getBoundingClientRect();
  if (!rect.width || !rect.height) return false;
  let left = 0,
    top = 0,
    right = innerWidth,
    bottom = innerHeight;
  for (let parent = node.parentElement; parent; parent = parent.parentElement) {
    const style = getComputedStyle(parent),
      box = parent.getBoundingClientRect();
    if (style.overflowX !== 'visible') {
      left = Math.max(left, box.left);
      right = Math.min(right, box.right);
    }
    if (style.overflowY !== 'visible') {
      top = Math.max(top, box.top);
      bottom = Math.min(bottom, box.bottom);
    }
  }
  return rect.right > left && rect.left < right && rect.bottom > top && rect.top < bottom;
}

/** Visibility admits only the displayed tile, never a hidden overflow image. The read
 * owner retains the verified display copy when a tile scrolls away and returns. */
function ImageTile({
  descriptor,
  url,
  error,
  load,
  open,
  single,
}: {
  descriptor: attachment.AttachmentDescriptor;
  url?: string;
  error?: string;
  load(visible: () => boolean): void;
  open(trigger: HTMLElement): void;
  single: boolean;
}) {
  const tile = useRef<HTMLButtonElement>(null);
  const visible = useRef(false);
  const read = useRef(load);
  read.current = load;
  useEffect(() => {
    const observer = new IntersectionObserver((entries) => {
      visible.current = entries.some((entry) => entry.isIntersecting);
      if (visible.current) read.current(() => visible.current && visibleTile(tile.current));
    });
    observer.observe(tile.current!);
    return () => observer.disconnect();
  }, []);
  return (
    <li className="message-attachment message-image" data-testid="message-attachment">
      <button
        ref={tile}
        type="button"
        className={single ? 'attachment-thumbnail single' : 'attachment-thumbnail'}
        aria-label={`Preview ${descriptor.filename}`}
        title={descriptor.filename}
        onClick={(event) => {
          if (event.isTrusted) open(event.currentTarget);
        }}
      >
        {url ? (
          <img src={url} alt={descriptor.filename} />
        ) : (
          <span>{error ?? text.attachmentOpening}</span>
        )}
      </button>
      {error && (
        <span className="attachment-state" role="alert">
          {error}
        </span>
      )}
    </li>
  );
}

/** Shared by Chat and thread turns. All remote bytes still cross the exact admitted
 * message-reference read; filenames are inert labels and raster types are rechecked. */
export function MessageAttachments({
  comment,
  binding,
}: {
  comment: CommentView;
  binding?: AttachmentBinding;
}) {
  const descriptors = comment.attachments ?? [];
  const reference = {
    kind: 'message',
    writer: comment.ref.writer,
    messageId: comment.messageId,
    revision: comment.revision,
  } as const;
  const referenceKey = JSON.stringify([reference, descriptors]);
  const opener = useAttachmentOpener(binding, referenceKey);
  const images = descriptors.filter(isRaster);
  const files = descriptors.filter((d) => !isRaster(d));
  const [viewer, setViewer] = useState<{ index: number; trigger: HTMLElement; lifetime: object }>();
  const active = viewer?.lifetime === opener.lifetime ? viewer : undefined;
  const show = (index: number, trigger: HTMLElement) => {
    void opener.preview(reference, images[index]);
    setViewer({ index, trigger, lifetime: opener.lifetime });
  };
  if (!descriptors.length) return null;
  const error = (id: string) => {
    const failure = opener.failed.get(id);
    return failure ? text.attachmentReason[failure] : undefined;
  };
  const displayed = images.length > 6 ? images.slice(0, 5) : images;
  return (
    <div
      className="message-attachments"
      aria-label={text.messageAttachmentsLabel}
      data-testid="message-attachments"
    >
      {!!images.length && (
        <ul className={images.length === 1 ? 'attachment-images single' : 'attachment-images'}>
          {displayed.map((d, index) => (
            <ImageTile
              key={`${referenceKey}:${d.attachmentId}:${opener.lifetime.generation}`}
              descriptor={d}
              single={images.length === 1}
              url={opener.shown[d.attachmentId]}
              error={error(d.attachmentId)}
              load={(visible) => {
                void opener.thumbnail(reference, d, visible);
              }}
              open={(trigger) => show(index, trigger)}
            />
          ))}
          {images.length > 6 && (
            <li>
              <button
                type="button"
                className="attachment-thumbnail attachment-more"
                aria-label={`Show ${images.length - 5} more images`}
                onClick={(event) => {
                  if (event.isTrusted) show(5, event.currentTarget);
                }}
              >
                +{images.length - 5}
              </button>
            </li>
          )}
        </ul>
      )}
      {!!files.length && (
        <ul className="attachment-files">
          {files.map((d) => {
            const type =
              d.filename.split('.').length > 1
                ? d.filename.split('.').at(-1)!.slice(0, 8).toUpperCase()
                : 'FILE';
            return (
              <li
                key={d.attachmentId}
                className="message-attachment message-file"
                data-testid="message-attachment"
              >
                <span className="attachment-type" aria-hidden="true">
                  {type}
                </span>
                <div>
                  <span className="attachment-name" title={d.filename}>
                    {d.filename}
                  </span>
                  <span className="attachment-size">
                    {type} · {text.attachmentSize(Number(d.plaintextBytes))}
                  </span>
                </div>
                <BrowserIconAction
                  type="button"
                  variant="text"
                  label={text.attachmentDownload}
                  icon={<Download />}
                  disabled={!binding || !!opener.busy}
                  onActivate={(event) => {
                    if (event.isTrusted) void opener.download(reference, d);
                  }}
                />
                {error(d.attachmentId) && (
                  <span className="attachment-state" role="alert">
                    {error(d.attachmentId)}
                  </span>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {active && (
        <AttachmentPreview
          images={images}
          index={active.index}
          trigger={active.trigger}
          url={opener.shown[images[active.index].attachmentId]}
          error={error(images[active.index].attachmentId)}
          select={(index) => show(index, active.trigger)}
          close={() => setViewer(undefined)}
          download={() => {
            void opener.download(reference, images[active.index]);
          }}
        />
      )}
    </div>
  );
}
