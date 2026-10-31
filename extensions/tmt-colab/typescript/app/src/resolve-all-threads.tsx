import { useEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { CheckCheck } from 'lucide-react';
import { BrowserAction, BrowserDialog, BrowserIconAction } from '@tmt/browser-ui/react';
import type { ThreadView } from './thread-records.js';
import type { ThreadStatusCoordinator } from './thread-status-coordinator.js';
import { AgentList } from './components/agent-chip.js';
import { useNotificationModalPresence } from './reply-notification-context.js';
import { text } from './strings.js';

/** Confirm a frozen filter snapshot; the coordinator owns admission and notification attempts. */
export function ResolveAllThreads({
  threads,
  coordinator,
  headerSlot,
  blocked,
  onReport,
}: {
  threads: readonly ThreadView[];
  coordinator?: Pick<ThreadStatusCoordinator, 'resolveAll'>;
  headerSlot: HTMLElement | null;
  blocked: boolean;
  onReport(value: ReactNode): void;
}) {
  const [confirmation, setConfirmation] = useState<readonly ThreadView[] | null>(null);
  const [busy, setBusy] = useState(false);
  const cancel = useRef<HTMLSpanElement>(null);
  const reportModal = useNotificationModalPresence();
  useEffect(() => {
    reportModal(confirmation !== null);
  }, [reportModal, confirmation]);
  const confirm = async () => {
    if (!coordinator || !confirmation || blocked || busy) return;
    const captured = confirmation;
    setConfirmation(null);
    setBusy(true);
    onReport(null);
    try {
      const result = await coordinator.resolveAll(captured);
      await result.settled;
      const failures = result.results.filter((value) => 'error' in value).length;
      const warnings = [
        ...new Set(
          result.results.flatMap((value) =>
            'result' in value ? (value.result.warnings?.map((warning) => warning.name) ?? []) : [],
          ),
        ),
      ];
      if (failures || warnings.length) {
        onReport(
          <>
            {failures > 0 && text.threadResolveAllFailed(failures)}
            {warnings.length > 0 && (
              <>
                {failures > 0 && ' '}
                {text.threadNotNotified} <AgentList names={warnings} />
              </>
            )}
          </>,
        );
      }
    } catch {
      onReport(text.commentFailed);
    } finally {
      setBusy(false);
    }
  };
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
            {null}
          </BrowserDialog>,
          document.body,
        )}
    </>
  );
}
