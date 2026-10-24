import { Diamond, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { BrowserAction, BrowserNotice } from '@tmt/browser-ui/react';
import { compactOffered, usageLevel, type CompactReport, type PageUsage } from './page-usage.js';
import { binarySize, text } from './strings.js';
import type { PageBinding } from './transport.js';

/** What a finished Compact tells the person (UX rulings on #2846): the words are the state. */
export type CompactOutcome =
  | { kind: 'combined'; merged: number; after: PageUsage; before: PageUsage }
  | { kind: 'still-high'; after: PageUsage }
  | { kind: 'nothing'; usage: PageUsage }
  | { kind: 'failed' };

/** The outcome of one report. Still high means combining kept every change and left the page at
 * 80% or more, or left only the stored size to limit it. */
export function outcomeOf(report: CompactReport): CompactOutcome {
  if (!report.compacted)
    return report.skipped === 'nothing-to-combine'
      ? { kind: 'nothing', usage: report.after }
      : { kind: 'failed' };
  return report.after.percent >= 80 || report.after.limiting === 'stored-size'
    ? { kind: 'still-high', after: report.after }
    : {
        kind: 'combined',
        merged: report.mergedObjects,
        after: report.after,
        before: report.before,
      };
}

export interface PageSize {
  /** Absent until the serve answered, or where this device cannot ask. */
  usage: PageUsage | null;
  outcome: CompactOutcome | null;
  compacting: boolean;
  /** Ask the serve again. Resolves to what it said, or null. */
  refresh(): Promise<PageUsage | null>;
  /** Combine, then show what happened. Resolves to the outcome; never saves or resends anything. */
  compact(): Promise<CompactOutcome>;
}
/** How full the page is, for a device that may save. A device without `usage` (a reader) gets
 * nothing, so no Compact is offered to it anywhere. */
export function usePageSize(binding: PageBinding | undefined): PageSize {
  const [usage, setUsage] = useState<PageUsage | null>(null);
  const [outcome, setOutcome] = useState<CompactOutcome | null>(null);
  const [compacting, setCompacting] = useState(false);
  const live = useRef(true);
  const busy = useRef(false);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  const refresh = useCallback(async () => {
    if (!binding?.usage) return null;
    try {
      const next = await binding.usage();
      if (live.current) setUsage(next);
      return next;
    } catch {
      // The page keeps working without the number; a lost connection has its own surface.
      return null;
    }
  }, [binding]);
  const compact = useCallback(async (): Promise<CompactOutcome> => {
    const failed: CompactOutcome = { kind: 'failed' };
    if (!binding?.compact || busy.current) return failed;
    busy.current = true;
    setCompacting(true);
    setOutcome(null);
    let result: CompactOutcome = failed;
    try {
      const report = await binding.compact();
      result = outcomeOf(report);
      if (live.current) setUsage(report.after);
    } catch {
      result = failed;
    } finally {
      busy.current = false;
      if (live.current) {
        setCompacting(false);
        setOutcome(result);
      }
    }
    return result;
  }, [binding]);
  return { usage, outcome, compacting, refresh, compact };
}

function share(used: number, limit: number): string {
  return text.pageSize.of(used.toLocaleString('en-US'), limit.toLocaleString('en-US'));
}
function otherDevices(usage: PageUsage) {
  return usage.otherDeviceUpdates > 0 ? (
    <p>{text.pageSize.otherDevices(usage.otherDeviceUpdates)}</p>
  ) : null;
}

/** The compact result line, or null while there is none. */
function OutcomeLine({ outcome, usage }: { outcome: CompactOutcome; usage: PageUsage | null }) {
  switch (outcome.kind) {
    case 'combined':
      return (
        <p role="status">
          {text.pageSize.combined(outcome.merged, outcome.after.percent, outcome.before.percent)}
        </p>
      );
    case 'still-high':
      return <p role="status">{text.pageSize.combinedStillHigh(outcome.after.percent)}</p>;
    case 'nothing':
      return (
        <>
          <p role="status">{text.pageSize.nothingToCombine}</p>
          {usage && otherDevices(usage)}
        </>
      );
    case 'failed':
      return <p role="status">{text.pageSize.failed}</p>;
  }
}

/** The one place for the action: Compact, its progress, its result, or why it is not offered. */
export function PageSizeActions({
  size,
  exportPage,
  why,
}: {
  size: PageSize;
  exportPage(): void;
  /** Say why Compact is not offered; the notice already says it in its title. */
  why: boolean;
}) {
  const { usage, outcome, compacting } = size;
  if (!usage) return null;
  const level = usageLevel(usage);
  const exportAction = (
    <BrowserAction
      type="button"
      label={text.pageSize.export}
      variant={why ? 'secondary' : 'text'}
      onActivate={() => exportPage()}
    />
  );
  const compactAction = (
    <BrowserAction
      type="button"
      label={text.pageSize.compact}
      variant={why ? 'secondary' : 'text'}
      onActivate={() => void size.compact()}
    />
  );
  if (compacting)
    return (
      <p role="status">
        <BrowserAction
          type="button"
          label={text.pageSize.compacting}
          variant="text"
          disabled
          onActivate={() => {}}
        />
      </p>
    );
  if (outcome)
    return (
      <>
        <OutcomeLine outcome={outcome} usage={usage} />
        {outcome.kind === 'still-high' && exportAction}
        {outcome.kind === 'failed' && compactAction}
      </>
    );
  if (compactOffered(usage)) return compactAction;
  if (level === 'ok') return null;
  return (
    <>
      {why && <p>{text.pageSize.cannotFree}</p>}
      {exportAction}
    </>
  );
}

/** The "Page size" section of the About drawer. */
export function PageSizeSection({ size, exportPage }: { size: PageSize; exportPage(): void }) {
  const { usage } = size;
  if (!usage) return null;
  const level = usageLevel(usage);
  return (
    <section className="page-size" aria-labelledby="page-size-heading" data-level={level}>
      <h3 id="page-size-heading">{text.pageSize.heading}</h3>
      <p>
        {level === 'full'
          ? text.pageSize.full(usage.percent)
          : level === 'nearly-full'
            ? text.pageSize.nearlyFull(usage.percent)
            : text.pageSize.percentFull(usage.percent)}
      </p>
      <dl>
        <dt>{text.pageSize.changes}</dt>
        <dd>{share(usage.updates, usage.updateLimit)}</dd>
        <dt>{text.pageSize.recentData}</dt>
        <dd>{text.pageSize.of(binarySize(usage.tailBytes), binarySize(usage.tailBytesLimit))}</dd>
        {usage.gzipBytes !== undefined && (
          <>
            <dt>{text.pageSize.storedSize}</dt>
            <dd>{text.pageSize.of(binarySize(usage.gzipBytes), binarySize(usage.gzipLimit))}</dd>
          </>
        )}
      </dl>
      {size.outcome?.kind === 'nothing' ? null : otherDevices(usage)}
      <PageSizeActions size={size} exportPage={exportPage} why />
    </section>
  );
}

/** The quiet notice at 80% or more: a state, so it cannot be dismissed, and it leaves when usage
 * drops below 80. Shown where people save. */
export function PageSizeNotice({ size, exportPage }: { size: PageSize; exportPage(): void }) {
  const { usage } = size;
  if (!usage || usageLevel(usage) === 'ok') return null;
  const full = usageLevel(usage) === 'full';
  const offered = compactOffered(usage);
  return (
    <div className="source-notice">
      <BrowserNotice
        tone={full ? 'blocked' : 'waiting'}
        announcement="status"
        stateLabel={full ? text.pageSize.noticeFull : text.pageSize.noticeNearly}
        title={offered ? text.pageSize.noticeUsed(usage.percent) : text.pageSize.noticeCannotFree}
        mark={full ? <X /> : <Diamond fill="currentColor" />}
        body={null}
        actions={<PageSizeActions size={size} exportPage={exportPage} why={false} />}
      />
    </div>
  );
}
