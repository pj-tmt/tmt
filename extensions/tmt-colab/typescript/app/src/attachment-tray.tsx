import { BrowserAction, BrowserIconAction } from '@tmt/browser-ui/react';
import { Paperclip } from 'lucide-react';
import { useRef, useSyncExternalStore } from 'react';
import type { AttachmentDraft, Chip } from './attachment-draft.js';
import { text } from './strings.js';
import './attachment-tray.css';

function stateText(chip: Chip): string {
  const state = chip.state;
  switch (state.kind) {
    case 'ready':
      return text.attachReady;
    case 'uploading':
      return text.attachUploading(Math.floor((state.sent / Math.max(state.total, 1)) * 100));
    case 'stored':
      return text.attachStored;
    case 'again':
      return state.why === 'page-changed' ? text.attachAgainPageChanged : text.attachAgainNotStored;
    case 'unknown':
      return text.attachUnknown;
    case 'refused':
      return text.attachRefused[state.reason];
  }
}

/** Files only join the draft here. Choosing one never sends, prepares an Ask, or touches the
 * message text; the parent's explicit Send uploads and publishes them with the message. */
export function AttachmentTray({ draft, disabled }: { draft: AttachmentDraft; disabled: boolean }) {
  const snapshot = useSyncExternalStore(draft.subscribe, draft.getSnapshot);
  const input = useRef<HTMLInputElement>(null);
  const action = (label: string, run: () => void, busy = false) => (
    <BrowserAction
      type="button"
      variant="text"
      label={label}
      disabled={disabled || busy}
      onActivate={(event) => {
        if (event.isTrusted) run();
      }}
    />
  );
  return (
    <div className="attachment-tray" data-testid="attachment-tray">
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
                {stateText(chip)}
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
          {text.attachNotice[notice.reason](notice.filename)}{' '}
          {action(text.attachDismiss, () => draft.dismiss(notice.id))}
        </p>
      ))}
    </div>
  );
}
