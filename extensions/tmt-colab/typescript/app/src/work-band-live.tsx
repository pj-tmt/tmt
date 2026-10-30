import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { BrowserMenu } from '@tmt/browser-ui/react';
import { text } from './strings.js';
import { SaveRefused, saveMessage } from './save.js';
import type { PageState } from './bootstrap.js';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { WorkBand, stateIcon, type WorkBandProps } from './work-band.js';
import { AddTask, AttachSubpage, TaskControls, useWorkRun } from './work-controls.js';
import type { PageBinding, PageTransport } from './transport.js';
import { useLinkedPages } from './work-stages.js';
import type { Work } from './work.js';

/** An ordinary page without work still has the manual state/progress line. */
const NO_WORK: Work = { version: 1, next: 1, order: [], tasks: {} };

/** The page's band with the owner's controls, where this device may change the work; anywhere
 * else it is the read-only band. */
export function PageWorkBand({
  binding,
  transport,
  space,
  pageId,
  ...band
}: Omit<WorkBandProps, 'work' | 'onOpenChange'> & {
  /** Absent for a page that never had work; its compact panel remains present. */
  work?: Work;
  binding?: PageBinding;
  /** Where a followed page's stage and this page's parent are read: the owner list this viewer
   * already has. */
  transport: PageTransport;
  space?: string;
  pageId: string;
}): ReactNode {
  const run = useWorkRun(binding);
  const [open, setOpen] = useState(false);
  const { stageOf, parent, subpages, observed, pages, knownChild } = useLinkedPages(
    transport,
    space,
    pageId,
    band.work,
    Boolean(open && binding?.work && binding.canWork?.()),
  );
  const childStates = JSON.stringify(subpages.map((child) => [child.id, child.state]));
  const previousChildStates = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!observed) return;
    const previous = previousChildStates.current;
    previousChildStates.current = childStates;
    if (previous !== undefined && previous !== childStates) binding?.refreshProgress?.();
  }, [binding, childStates, observed]);
  const work = band.work ?? NO_WORK;
  const stateControl = <PageStateMenu binding={binding} state={band.pageState} />;
  if (!binding?.work || !binding.canWork?.())
    return (
      <WorkBand
        {...band}
        work={work}
        onOpenChange={setOpen}
        stateControl={stateControl}
        parent={parent}
        subpages={subpages}
        stageOf={stageOf}
      />
    );
  return (
    <WorkBand
      {...band}
      work={work}
      onOpenChange={setOpen}
      stateControl={stateControl}
      attachAction={
        observed && pages?.find((page) => page.id === pageId)?.summary && !knownChild ? (
          <AttachSubpage pages={pages} pageId={pageId} work={work} run={run} />
        ) : undefined
      }
      parent={parent}
      subpages={subpages}
      stageOf={stageOf}
      actions={(task, id, state) => (
        <TaskControls
          key={id}
          id={id}
          task={task}
          state={state}
          work={run}
          pageHref={band.pageHref}
        />
      )}
      listActions={<AddTask work={run} />}
    />
  );
}

/** The verified label moves only when the owner's admitted stream publishes the commit. */
function PageStateMenu({ binding, state }: { binding?: PageBinding; state?: PageState | null }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const revision = useRef<Promise<string | undefined>>(undefined);
  const label = state ? text.workBand.taskState[state] : text.workBand.stateUnavailable;
  if (!binding?.canMetadata?.() || !binding.metadata || !binding.metadataRevision || state == null)
    return <span className={`work-page-state ${ui.countChip}`}>{label}</span>;
  const select = async (next: string) => {
    if (busy || !['todo', 'in-progress', 'done'].includes(next)) return;
    setBusy(true);
    setError(undefined);
    try {
      const frozen = await revision.current;
      if (frozen === undefined) return;
      await binding.metadata!('page.state', { state: next }, frozen);
    } catch (failure) {
      setError(failure instanceof SaveRefused ? failure.message : saveMessage(failure));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="work-state-control">
      <BrowserMenu
        label={text.workBand.pageState}
        variant="count"
        valueLabel={label}
        tone={state === 'in-progress' ? 'attention' : 'neutral'}
        icon={stateIcon(state, 'tmt-ui-icon')}
        disabled={busy}
        onOpenChange={(open) => {
          if (!open) return;
          setError(undefined);
          revision.current = binding.metadataRevision!({ pageState: state }, 'page-state').catch(
            (failure) => {
              setError(failure instanceof Error ? failure.message : saveMessage(failure));
              return undefined;
            },
          );
        }}
        items={(['todo', 'in-progress', 'done'] as const).map((value) => ({
          key: value,
          label: text.workBand.taskState[value],
          role: 'menuitemradio' as const,
          checked: value === state,
        }))}
        onSelect={(key) => {
          void select(key);
        }}
      />
      {busy && <span role="status">{text.workBand.savingState}</span>}
      {error && <span role="alert">{error}</span>}
    </div>
  );
}
