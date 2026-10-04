import { useEffect, useRef, useState } from 'react';
import { DISCLOSURE, Downloads, type ExportFile } from './export.js';
import type { PageBinding } from './transport.js';
import { PageDrawer } from './page-drawer.js';
import { text } from './strings.js';

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
  const current = useRef<Downloads | null>(null);
  useEffect(() => {
    if (!open || !binding || blocked) return;
    let active = true;
    setState('preparing');
    setRequested([]);
    void binding.export().then(
      (bundle) => {
        if (!active) return;
        current.current = new Downloads(bundle);
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
            : requested.length === 1
              ? text.exportPartial
              : requested.length === 2
                ? text.exportRequested
                : text.exportReady}
      </p>
      <div className="export-actions">
        {(['page.html', 'manifest.json'] as const).map((name) => (
          <button
            key={name}
            disabled={blocked || state !== 'ready'}
            onClick={(event) => {
              if (event.isTrusted) download(name);
            }}
          >
            {text.download} {name}
            {requested.includes(name) ? ' ✓' : ''}
          </button>
        ))}
        <button onClick={() => setOpen(false)}>{text.exportClose}</button>
      </div>
    </section>
  );
  return (
    <div className="export-control">
      <button
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
