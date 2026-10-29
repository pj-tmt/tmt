import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { CheckCheck } from 'lucide-react';
import { BrowserAction, BrowserDialog, BrowserIconAction } from '@tmt/browser-ui/react';
import type { ThreadView } from './thread-records.js';
import type { ResolveAllResult, ThreadStatusCoordinator } from './thread-status-coordinator.js';
import { AgentList } from './components/agent-chip.js';
import { useNotificationModalPresence } from './reply-notification-context.js';
import { text } from './strings.js';

/** The page owns the filter and admission. This action owns its confirmation snapshot and report;
 * committed statuses and notification attempts stay with the existing coordinator. */
export function ResolveAllThreads({
  threads,
  coordinator,
  headerSlot,
  blocked,
}: {
  threads: readonly ThreadView[];
  coordinator?: Pick<ThreadStatusCoordinator, 'resolveAll'>;
  headerSlot: HTMLElement | null;
  blocked: boolean;
}) {
  const [confirmation, setConfirmation] = useState<readonly ThreadView[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<ResolveAllResult>();
  const [error, setError] = useState(false);
  const cancel = useRef<HTMLSpanElement>(null);
  const report = useNotificationModalPresence();
  const confirming = confirmation !== null;
  useEffect(() => {
    report(confirming);
  }, [report, confirming]);
  const confirm = async () => {
    if (!coordinator || !confirmation || blocked || busy) return;
    const captured = confirmation;
    setConfirmation(null);
    setBusy(true);
    setResult(undefined);
    setError(false);
    try {
      const report = await coordinator.resolveAll(captured);
      setResult(report);
      await report.settled;
    } catch {
      setError(true);
    } finally {
      setBusy(false);
    }
  };
  const changes =
    result?.results.flatMap((value) => ('result' in value ? [value.result] : [])) ?? [];
  const warnings = [
    ...new Set(changes.flatMap((value) => value.warnings?.map((warning) => warning.name) ?? [])),
  ];
  return (
    <>
      {coordinator &&
        headerSlot &&
        createPortal(
          <BrowserIconAction
            type="button"
            variant="text"
            label={text.threadResolveAll(threads.length)}
            icon={<CheckCheck />}
            disabled={blocked || busy || threads.length === 0}
            busy={busy}
            onActivate={(event) => {
              if (event.isTrusted && !blocked && !busy)
                setConfirmation(structuredClone([...threads]));
            }}
          />,
          headerSlot,
        )}
      {confirmation &&
        createPortal(
          <BrowserDialog
            open
            presentation="confirm"
            title={text.threadResolveAllConfirm(confirmation.length)}
            initialFocus={() => cancel.current?.querySelector('button') ?? null}
            onDismiss={() => setConfirmation(null)}
            actions={
              <>
                <span ref={cancel}>
                  <BrowserAction
                    type="button"
                    variant="secondary"
                    label={text.commentCancel}
                    onActivate={(event) => {
                      if (event.isTrusted) setConfirmation(null);
                    }}
                  />
                </span>
                <BrowserAction
                  type="button"
                  variant="primary"
                  label={text.threadResolveAllCommit}
                  disabled={blocked || !coordinator}
                  onActivate={(event) => {
                    if (event.isTrusted) void confirm();
                  }}
                />
              </>
            }
          >
            <p>{text.threadResolveAllBody}</p>
          </BrowserDialog>,
          document.body,
        )}
      {result && (
        <p role="status" className="annotation-hint resolve-all-report">
          {text.threadResolveAllResult(
            changes.filter((value) => value.changed).length,
            result.results.length - changes.length,
          )}
          {warnings.length > 0 && (
            <>
              {' '}
              {text.threadNotNotified} <AgentList names={warnings} />
            </>
          )}
        </p>
      )}
      {error && (
        <p role="alert" className="resolve-all-report">
          {text.commentFailed}
        </p>
      )}
    </>
  );
}
