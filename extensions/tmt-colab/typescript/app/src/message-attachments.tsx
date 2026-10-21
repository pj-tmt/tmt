import { visibleElement } from './visible-element.js';
import { BrowserIconAction } from '@tmt/browser-ui/react';
import { Download } from 'lucide-react';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { RASTER_TYPES } from './attachment-file.js';
import type { EchoAttachment } from './message-echo.js';
import { isRaster, useAttachmentOpener } from './attachment-opener.js';
import { AttachmentPreview } from './attachment-preview.js';
import type { AttachmentBinding } from './attachment-service.js';
import { text } from './strings.js';
import { AttachmentLabel } from './attachment-tray.js';
import type { CommentView } from './thread-records.js';
import './attachment-tray.css';

/** Visibility admits only the displayed tile, never a hidden overflow image. The read
 * owner retains the verified display copy when a tile scrolls away and returns. */
function ImageTile({
  item,
  url,
  error,
  load,
  open,
  single,
}: {
  item: { filename: string; mediaType: string; size: number };
  url?: string;
  error?: string;
  load?(visible: () => boolean): void;
  open?(trigger: HTMLElement): void;
  single: boolean;
}) {
  const tile = useRef<HTMLButtonElement>(null);
  const visible = useRef(false);
  const read = useRef(load);
  read.current = load;
  useEffect(() => {
    if (!read.current) return;
    const observer = new IntersectionObserver((entries) => {
      visible.current = entries.some((entry) => entry.isIntersecting);
      if (visible.current) read.current?.(() => visible.current && visibleElement(tile.current));
    });
    observer.observe(tile.current!);
    return () => observer.disconnect();
  }, []);
  return (
    <li className="message-attachment message-image" data-testid="message-attachment">
      <AttachmentLabel filename={item.filename} mediaType={item.mediaType} size={item.size} />
      <button
        ref={tile}
        disabled={!open}
        type="button"
        className={single ? 'attachment-thumbnail single' : 'attachment-thumbnail'}
        aria-label={`Preview ${item.filename}`}
        title={item.filename}
        onClick={(event) => {
          if (event.isTrusted) open?.(event.currentTarget);
        }}
      >
        {url ? (
          <img src={url} alt={item.filename} />
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

/** One layout for pending display copies and admitted message tiles. */
function AttachmentTiles({
  images,
  files,
  more,
  notice,
  children,
}: {
  images: ReactNode[];
  files: ReactNode[];
  more?(trigger: HTMLElement): void;
  notice?: ReactNode;
  children?: ReactNode;
}) {
  if (!images.length && !files.length) return children ?? null;
  return (
    <div
      className="message-attachments"
      aria-label={text.messageAttachmentsLabel}
      data-testid="message-attachments"
    >
      {notice}
      {!!images.length && (
        <ul className={images.length === 1 ? 'attachment-images single' : 'attachment-images'}>
          {images.length > 6 ? images.slice(0, 5) : images}
          {images.length > 6 && (
            <li>
              <button
                type="button"
                className="attachment-thumbnail attachment-more"
                aria-label={`Show ${images.length - 5} more images`}
                disabled={!more}
                onClick={(event) => {
                  if (event.isTrusted) more?.(event.currentTarget);
                }}
              >
                +{images.length - 5}
              </button>
            </li>
          )}
        </ul>
      )}
      {!!files.length && <ul className="attachment-files">{files}</ul>}
      {children}
    </div>
  );
}

function FileTile({
  item,
  disabled = false,
  download,
  error,
}: {
  item: { filename: string; mediaType: string; size: number };
  disabled?: boolean;
  download?(): void;
  error?: string;
}) {
  return (
    <li className="message-attachment message-file" data-testid="message-attachment">
      <AttachmentLabel filename={item.filename} mediaType={item.mediaType} size={item.size} />
      <BrowserIconAction
        type="button"
        variant="text"
        label={text.attachmentDownload}
        icon={<Download />}
        disabled={disabled || !download}
        onActivate={(event) => {
          if (event.isTrusted) download?.();
        }}
      />
      {error && (
        <span className="attachment-state" role="alert">
          ✗ {error}
        </span>
      )}
    </li>
  );
}

function EchoImage({ item, single }: { item: EchoAttachment; single: boolean }) {
  const [url, setUrl] = useState<string>();
  useEffect(() => {
    let current = true;
    void item.preview.then((value) => {
      if (current) setUrl(value);
    });
    return () => {
      current = false;
    };
  }, [item]);
  return <ImageTile item={item} single={single} url={url} />;
}

/** Local display only, with no remote read or attachment actions. The failed row keeps
 * these captured tiles while its composer owns the retryable draft. */
export function EchoAttachments({ attachments }: { attachments: readonly EchoAttachment[] }) {
  const images = attachments.filter((item) => RASTER_TYPES.some((type) => type === item.mediaType));
  const files = attachments.filter((item) => !RASTER_TYPES.some((type) => type === item.mediaType));
  return (
    <AttachmentTiles
      images={images.map((item) => (
        <EchoImage key={item.id} item={item} single={images.length === 1} />
      ))}
      files={files.map((item) => (
        <FileTile key={item.id} item={item} />
      ))}
    />
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
  const opener = useAttachmentOpener(binding, reference, descriptors);
  const images = descriptors.filter(isRaster);
  const files = descriptors.filter((d) => !isRaster(d));
  const [viewer, setViewer] = useState<{ index: number; trigger: HTMLElement; lifetime: object }>();
  const active = viewer?.lifetime === opener.lifetime ? viewer : undefined;
  const show = (index: number, trigger: HTMLElement) => {
    void opener.preview(images[index]);
    setViewer({ index, trigger, lifetime: opener.lifetime });
  };
  if (!descriptors.length)
    return opener.unlistedFailure ? <p role="alert">{text.attachmentReason.changed}</p> : null;
  const error = (id: string) => {
    const failure = opener.failed.get(id);
    return failure ? text.attachmentReason[failure] : undefined;
  };
  return (
    <AttachmentTiles
      images={images.map((d, index) => (
        <ImageTile
          key={`${referenceKey}:${d.attachmentId}:${opener.lifetime.generation}`}
          item={{ filename: d.filename, mediaType: d.mediaType, size: Number(d.plaintextBytes) }}
          single={images.length === 1}
          url={opener.shown[d.attachmentId]}
          error={error(d.attachmentId)}
          load={(visible) => {
            void opener.thumbnail(d, visible);
          }}
          open={(trigger) => show(index, trigger)}
        />
      ))}
      files={files.map((d) => (
        <FileTile
          key={d.attachmentId}
          item={{ filename: d.filename, mediaType: d.mediaType, size: Number(d.plaintextBytes) }}
          disabled={!binding || !!opener.busy}
          download={() => {
            void opener.download(d);
          }}
          error={error(d.attachmentId)}
        />
      ))}
      more={(trigger) => show(5, trigger)}
      notice={opener.unlistedFailure && <p role="alert">{text.attachmentReason.changed}</p>}
    >
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
            void opener.download(images[active.index]);
          }}
        />
      )}
    </AttachmentTiles>
  );
}
