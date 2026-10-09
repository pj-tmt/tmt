import { BrowserAction, BrowserIconAction } from '@tmt/browser-ui/react';
import { Paperclip } from 'lucide-react';
import { useRef, useSyncExternalStore } from 'react';
import type { AttachmentDraft, Chip } from './attachment-draft.js';
import { text } from './strings.js';
import './attachment-tray.css';

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
      return text.attachRefused[state.reason];
  }
}

/** Files only join the draft here. Choosing one never sends, prepares an Ask, or touches the
 * message text; the parent's explicit Send uploads and publishes them with the message. */
export function AttachButton({
  draft,
  disabled,
  labeled = false,
}: {
  draft: AttachmentDraft;
  disabled: boolean;
  /** Icon plus the visible words, where the panel has no other cue (the Files panel). */
  labeled?: boolean;
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
            label={text.attachFiles}
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

/** The chips and refusal notices of one composer, above its status row. */
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
