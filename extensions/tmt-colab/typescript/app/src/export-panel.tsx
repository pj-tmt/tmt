import { BrowserAction, BrowserList, BrowserListRow } from '@tmt/browser-ui/react';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { Check } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import {
  DISCLOSURE,
  Downloads,
  PAGE_FILES,
  type ExportFile,
  type ListedAttachment,
} from './export.js';
import type { PageBinding } from './transport.js';
import { PageDrawer } from './page-drawer.js';
import { text } from './strings.js';
import './attachment-tray.css';

/** Downloads stay in trusted chrome; no renderer message opens this panel. */
export function ExportPanel({
  binding,
  blocked,
  drawer = false,
  opened,
  changeOpen,
}: {
  binding?: PageBinding;
  blocked: boolean;
  drawer?: boolean;
  opened?: boolean;
  changeOpen?(open: boolean): void;
}) {
  const [localOpen, setLocalOpen] = useState(false);
  const open = opened ?? localOpen;
  const setOpen = changeOpen ?? setLocalOpen;
  const [state, setState] = useState<'preparing' | 'ready' | 'failed'>('preparing');
  const [requested, setRequested] = useState<ExportFile[]>([]);
  // Each attachment of this copy with its outcome; the page files are always the same four.
  const [attachments, setAttachments] = useState<readonly ListedAttachment[]>([]);
  const current = useRef<Downloads | null>(null);
  useEffect(() => {
    if (!open || !binding || blocked) return;
    let active = true;
    setState('preparing');
    setRequested([]);
    setAttachments([]);
    void binding.export().then(
      (bundle) => {
        if (!active) return;
        current.current = new Downloads(bundle);
        setAttachments(bundle.attachments);
        setState('ready');
      },
      () => {
        if (active) setState('failed');
      },
    );
    return () => {
      active = false;
      current.current?.close();
      current.current = null;
    };
  }, [binding, open, blocked]);
  useEffect(() => {
    if (blocked && open) setOpen(false);
  }, [blocked, open, setOpen]);
  function download(name: ExportFile) {
    try {
      if (!current.current || blocked) return;
      current.current.request(name);
      setRequested((previous) => (previous.includes(name) ? previous : [...previous, name]));
    } catch {
      setState('failed');
    }
  }
  const downloadable = PAGE_FILES.length + attachments.filter((a) => a.file).length;
  const content = (
    <section className="export-panel" aria-label={text.export}>
      {!drawer && <h2>{text.export}</h2>}
      <p>{DISCLOSURE}</p>
      <p>{text.exportDiscussions}</p>
      <p role="status">
        {state === 'preparing'
          ? text.exportPreparing
          : state === 'failed'
            ? text.exportFailed
            : requested.length === downloadable
              ? text.exportRequested
              : requested.length > 0
                ? text.exportPartial
                : text.exportReady}
      </p>
      <div className="export-actions">
        {PAGE_FILES.map((name) => (
          <button
            key={name}
            disabled={blocked || state !== 'ready'}
            onClick={(event) => {
              if (event.isTrusted) download(name);
            }}
          >
            {text.download} {name}
            {requested.includes(name) && <Check aria-hidden />}
          </button>
        ))}
        <button onClick={() => setOpen(false)}>{text.exportClose}</button>
      </div>
      {attachments.length > 0 && (
        <section className="export-attachments" aria-label={text.exportAttachments}>
          <h3>{text.exportAttachments}</h3>
          <BrowserList label={text.exportAttachments} className="files-list">
            {attachments.map((item) => (
              <BrowserListRow
                key={item.attachmentId}
                title={item.filename}
                metadata={
                  item.file === undefined
                    ? text.exportAttachmentState[item.reason ?? 'missing']
                    : text.attachmentSize(item.plaintextBytes)
                }
                state={null}
                actions={
                  item.file === undefined ? undefined : (
                    <BrowserAction
                      type="button"
                      variant="text"
                      label={
                        requested.includes(item.file)
                          ? text.exportAttachmentRequested
                          : text.attachmentDownload
                      }
                      disabled={blocked || state !== 'ready'}
                      onActivate={(event) => {
                        if (event.isTrusted) download(item.file!);
                      }}
                    />
                  )
                }
              />
            ))}
          </BrowserList>
        </section>
      )}
    </section>
  );
  return (
    <div className="export-control">
      <button
        className={ui.action}
        data-variant="text"
        disabled={!binding || blocked}
        title={!binding ? text.exportUnavailable : undefined}
        onClick={(event) => {
          if (event.isTrusted) setOpen(!open);
        }}
        aria-expanded={open}
      >
        {text.export}
      </button>
      {drawer ? (
        <PageDrawer open={open} title={text.export} kind="export" close={() => setOpen(false)}>
          {open && content}
        </PageDrawer>
      ) : (
        open && content
      )}
    </div>
  );
}
