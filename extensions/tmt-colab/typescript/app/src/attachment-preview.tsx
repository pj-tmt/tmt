import { BrowserIconAction } from '@tmt/browser-ui/react';
import { ArrowLeft, ArrowRight, Download, X } from 'lucide-react';
import { useEffect, useId, useRef } from 'react';
import { createPortal } from 'react-dom';
import type { attachment } from '@tmt/colab-client';
import { text } from './strings.js';
import './attachment-preview.css';

/** Native modality owns tab containment. The original tile owns return focus, even when
 * navigation changes the viewed image. No URL or filename grants read authority. */
export function AttachmentPreview({
  images,
  index,
  url,
  error,
  trigger,
  select,
  close,
  download,
}: {
  images: readonly attachment.AttachmentDescriptor[];
  index: number;
  url?: string;
  error?: string;
  trigger: HTMLElement;
  select(index: number): void;
  close(): void;
  download(): void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const label = useId();
  const closeButton = useRef<HTMLButtonElement>(null);
  const image = images[index];
  useEffect(() => {
    const node = dialog.current!;
    const conversation = trigger.closest<HTMLElement>('.conversation-turn');
    node.showModal();
    closeButton.current?.focus();
    return () => {
      node.close();
      if (trigger.isConnected && trigger.getClientRects().length)
        trigger.focus({ preventScroll: true });
      else if (conversation?.isConnected)
        conversation
          .querySelector<HTMLButtonElement>('button:not(:disabled)')
          ?.focus({ preventScroll: true });
      else document.querySelector<HTMLButtonElement>('.page-header-actions button')?.focus();
    };
  }, [trigger]);
  return createPortal(
    <dialog
      ref={dialog}
      className="attachment-viewer"
      aria-labelledby={label}
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
      onKeyDown={(event) => {
        if (!event.isTrusted) return;
        if (event.key === 'Escape') {
          event.preventDefault();
          event.stopPropagation();
          close();
        }
        if (event.key === 'ArrowLeft' && index > 0) {
          event.preventDefault();
          select(index - 1);
        }
        if (event.key === 'ArrowRight' && index < images.length - 1) {
          event.preventDefault();
          select(index + 1);
        }
      }}
    >
      <header className="attachment-viewer-header">
        <div>
          <h2 id={label}>{image.filename}</h2>
          <span>
            {text.attachmentSize(Number(image.plaintextBytes))} · {index + 1} / {images.length}
          </span>
        </div>
        <BrowserIconAction
          type="button"
          variant="text"
          label={text.attachmentDownload}
          icon={<Download />}
          onActivate={(event) => {
            if (event.isTrusted) download();
          }}
        />
        <BrowserIconAction
          type="button"
          variant="text"
          label="Close preview"
          buttonRef={closeButton}
          icon={<X />}
          onActivate={(event) => {
            if (event.isTrusted) close();
          }}
        />
      </header>
      <div className="attachment-viewer-image">
        {url ? (
          <img src={url} alt={image.filename} />
        ) : (
          <p role={error ? 'alert' : 'status'}>{error ?? text.attachmentOpening}</p>
        )}
      </div>
      <footer>
        <BrowserIconAction
          type="button"
          variant="text"
          label="Previous image"
          icon={<ArrowLeft />}
          disabled={index === 0}
          onActivate={(event) => {
            if (event.isTrusted) select(index - 1);
          }}
        />
        <BrowserIconAction
          type="button"
          variant="text"
          label="Next image"
          icon={<ArrowRight />}
          disabled={index === images.length - 1}
          onActivate={(event) => {
            if (event.isTrusted) select(index + 1);
          }}
        />
      </footer>
    </dialog>,
    document.body,
  );
}
