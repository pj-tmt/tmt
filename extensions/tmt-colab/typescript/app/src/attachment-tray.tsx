import { BrowserAction, BrowserIconAction } from '@tmt/browser-ui/react';
import { FileText, Image as ImageIcon, Paperclip, X } from 'lucide-react';
import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import type { AttachmentDraft, Chip } from './attachment-draft.js';
import { text } from './strings.js';
import './attachment-tray.css';

/** Inert metadata shared by pending chips and verified message attachments. */
export function AttachmentLabel({
  filename,
  mediaType,
  size,
}: {
  filename: string;
  mediaType: string;
  size: number;
}) {
  const image = mediaType.startsWith('image/');
  const Icon = image ? ImageIcon : FileText;
  const type =
    mediaType === 'text/plain'
      ? 'Text'
      : mediaType === 'application/octet-stream'
        ? 'File'
        : (mediaType.split('/').at(-1) ?? 'File').toUpperCase();
  return (
    <span className="attachment-label">
      <Icon aria-hidden />
      <span>
        <span className="attachment-name" title={filename}>
          {filename}
        </span>
        <span className="attachment-size">
          {type} · {text.attachmentSize(size)}
        </span>
      </span>
    </span>
  );
}

function stateText(chip: Chip, page: boolean): string {
  const state = chip.state;
  switch (state.kind) {
    case 'ready':
      return page ? text.filesReady : text.attachReady;
    case 'uploading':
      return text.attachUploading(Math.floor((state.sent / Math.max(state.total, 1)) * 100));
    case 'stored':
      return text.attachStored;
    case 'again':
      return state.why === 'page-changed'
        ? page
          ? text.filesAgainPageChanged
          : text.attachAgainPageChanged
        : page
          ? text.filesAgainNotStored
          : text.attachAgainNotStored;
    case 'unknown':
      return text.attachUnknown;
    case 'refused':
      return state.reason === 'capacity'
        ? text.storageFull(state.usage)
        : text.attachRefused[state.reason];
  }
}

/** Files only join the draft here. Choosing one never sends, prepares an Ask, or touches the
 * message text; the parent's explicit Send uploads and publishes them with the message. */
export function AttachButton({
  draft,
  disabled,
  labeled = false,
  label = text.attachFiles,
}: {
  draft: AttachmentDraft;
  disabled: boolean;
  /** Icon plus visible words in the composer or Files panel. */
  labeled?: boolean;
  label?: string;
}) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <>
      <input
        ref={input}
        type="file"
        multiple
        hidden
        data-testid="attachment-input"
        tabIndex={-1}
        onChange={(event) => {
          const files = [...(event.currentTarget.files ?? [])];
          event.currentTarget.value = '';
          if (files.length) void draft.add(files);
        }}
      />
      {labeled ? (
        <span className="attach-labeled">
          <Paperclip aria-hidden />
          <BrowserAction
            type="button"
            variant="text"
            label={label}
            disabled={disabled}
            onActivate={(event) => {
              if (event.isTrusted) input.current?.click();
            }}
          />
        </span>
      ) : (
        <BrowserIconAction
          type="button"
          variant="text"
          label={text.attachFiles}
          icon={<Paperclip />}
          disabled={disabled}
          onActivate={(event) => {
            if (event.isTrusted) input.current?.click();
          }}
        />
      )}
    </>
  );
}

function LocalPreview({ draft, chip }: { draft: AttachmentDraft; chip: Chip }) {
  const [url, setUrl] = useState<string>();
  useEffect(() => {
    let current = true;
    void draft.preview(chip.id).then((url) => {
      if (current) setUrl(url);
    });
    return () => {
      current = false;
    };
  }, [draft, chip.id]);
  return url ? (
    <span className="attachment-pending-preview">
      <img className="attachment-pending-image" src={url} alt={chip.filename} />
    </span>
  ) : null;
}

/** A presentation row inside the input, outside Lexical. Draft remains the only owner. */
export function PendingAttachments({
  draft,
  disabled,
  onRemoved,
}: {
  draft: AttachmentDraft;
  disabled: boolean;
  onRemoved(): void;
}) {
  const snapshot = useSyncExternalStore(draft.subscribe, draft.getSnapshot);
  const action = (label: string, run: () => void) => (
    <BrowserAction
      type="button"
      variant="text"
      label={label}
      disabled={disabled}
      onActivate={(event) => {
        if (event.isTrusted) run();
      }}
    />
  );
  if (!snapshot.chips.length && !snapshot.notices.length) return null;
  return (
    <div className="attachment-pending" data-testid="attachment-tray">
      {snapshot.chips.length > 0 && (
        <ul className="attachment-pending-row" aria-label={text.attachListLabel}>
          {snapshot.chips.map((chip) => (
            <li
              key={chip.id}
              className="attachment-pending-item"
              data-testid="attachment-chip"
              data-state={chip.state.kind}
            >
              <AttachmentLabel
                filename={chip.filename}
                mediaType={chip.mediaType}
                size={chip.size}
              />
              <span className="attachment-state" role="status">
                {chip.state.kind === 'refused' ? '✗ ' : ''}
                {stateText(chip, false)}
              </span>
              {chip.state.kind === 'unknown' &&
                action(text.attachCheck, () => void draft.check(chip.id))}
              {chip.state.kind === 'refused' &&
                action(text.attachRetry, () => draft.retry(chip.id))}
              <BrowserIconAction
                type="button"
                variant="text"
                label={`${text.attachRemove} ${chip.filename}`}
                tooltipLabel="Remove"
                icon={<X />}
                disabled={disabled || chip.state.kind === 'uploading'}
                onActivate={(event) => {
                  if (event.isTrusted) {
                    draft.remove(chip.id);
                    onRemoved();
                  }
                }}
              />
              <LocalPreview draft={draft} chip={chip} />
            </li>
          ))}
        </ul>
      )}
      {snapshot.notices.map((notice) => (
        <p
          key={notice.id}
          className="attachment-notice"
          role="status"
          data-testid="attachment-notice"
        >
          {text.attachNotice[notice.reason](notice.filename)}{' '}
          {action(text.attachDismiss, () => draft.dismiss(notice.id))}
        </p>
      ))}
    </div>
  );
}

/** Files-panel upload rows and refusal notices; message drafts use PendingAttachments. */
export function AttachmentChips({
  draft,
  disabled,
  page = false,
}: {
  draft: AttachmentDraft;
  disabled: boolean;
  /** Chips of the page's Files panel: they upload when added, not when a message is sent. */
  page?: boolean;
}) {
  const snapshot = useSyncExternalStore(draft.subscribe, draft.getSnapshot);
  const action = (label: string, run: () => void) => (
    <BrowserAction
      type="button"
      variant="text"
      label={label}
      disabled={disabled}
      onActivate={(event) => {
        if (event.isTrusted) run();
      }}
    />
  );
  if (!snapshot.chips.length && !snapshot.notices.length) return null;
  return (
    <div className="attachment-tray" data-testid="attachment-tray">
      {snapshot.chips.length > 0 && (
        <ul className="attachment-chips" aria-label={text.attachListLabel}>
          {snapshot.chips.map((chip) => (
            <li
              key={chip.id}
              className="attachment-chip"
              data-testid="attachment-chip"
              data-state={chip.state.kind}
            >
              <span className="attachment-name">{chip.filename}</span>
              <span className="attachment-size">{text.attachmentSize(chip.size)}</span>
              <span className="attachment-state" role="status">
                {stateText(chip, page)}
              </span>
              <span className="attachment-actions">
                {chip.state.kind === 'unknown' &&
                  action(text.attachCheck, () => void draft.check(chip.id))}
                {chip.state.kind === 'refused' &&
                  action(text.attachRetry, () => draft.retry(chip.id))}
                {chip.state.kind !== 'uploading' &&
                  action(text.attachRemove, () => draft.remove(chip.id))}
              </span>
            </li>
          ))}
        </ul>
      )}
      {snapshot.notices.map((notice) => (
        <p
          key={notice.id}
          className="attachment-notice"
          role="status"
          data-testid="attachment-notice"
        >
          {page && notice.reason === 'too-many'
            ? text.filesTooMany(notice.filename)
            : text.attachNotice[notice.reason](notice.filename)}{' '}
          {action(text.attachDismiss, () => draft.dismiss(notice.id))}
        </p>
      ))}
    </div>
  );
}
