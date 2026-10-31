import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { BrowserMenu } from '@tmt/browser-ui/react';
import { MoreHorizontal } from 'lucide-react';
import { text } from './strings.js';
import { SaveNotApplied, SaveRefused, saveMessage, type MetadataAction } from './save.js';
import type { PageState } from './bootstrap.js';
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { WorkBand, type WorkBandProps } from './work-band.js';
import { AddSubpageDialog, type AddChoice } from './work-add-dialog.js';
import type { PageBinding, PageTransport } from './transport.js';
import { rememberPages } from './quick-switcher-source.js';
import { useLinkedPages } from './work-stages.js';
import { subpageTree, type SubpageRow } from './work-tree.js';
import type { Work } from './work.js';

const PAGE_STATES = ['todo', 'in-progress', 'done'] as const;
const t = text.workBand;

/** A refusal the serve explained, as opposed to an outcome nobody knows. */
const explained = (failure: unknown) =>
  failure instanceof SaveRefused || failure instanceof SaveNotApplied;

/** Run one owner write on another page: open its binding, write, give the connection up. */
async function onPage<T>(
  transport: PageTransport,
  pageId: string,
  use: (binding: PageBinding, shown: PageState | null | undefined) => Promise<T>,
): Promise<T> {
  const snapshot = await transport.page(pageId);
  try {
    if (!snapshot.binding?.canWork?.()) throw new Error('This page cannot be changed here.');
    return await use(snapshot.binding, snapshot.pageState ?? snapshot.state);
  } finally {
    snapshot.binding?.close();
  }
}

/** Set a page's manual state through its expected-revision metadata route. */
async function writeState(binding: PageBinding, shown: PageState | null | undefined, next: string) {
  if (!binding.canMetadata?.() || !binding.metadata || !binding.metadataRevision)
    throw new Error('Page state cannot be changed here.');
  const revision = await binding.metadataRevision({ pageState: shown ?? 'todo' }, 'page-state');
  await binding.metadata('page.state' satisfies MetadataAction, { state: next }, revision);
}

/** The page's band with the owner's controls, where this device may change the work; anywhere
 * else it is the read-only band. */
export function PageWorkBand({
  binding,
  transport,
  space,
  pageId,
  work,
  title,
  ...band
}: Omit<WorkBandProps, 'subpages' | 'childrenOf' | 'counts' | 'stateControl'> & {
  /** Absent for a page that never had work. */
  work?: Work;
  title: string;
  binding?: PageBinding;
  /** Where sub-page titles, states and deeper levels are read: the owner list this viewer
   * already has. */
  transport: PageTransport;
  space?: string;
  pageId: string;
}): ReactNode {
  const { observed, pages } = useLinkedPages(transport, space, work);
  const tree = useMemo(() => subpageTree(pageId, work, pages), [pageId, work, pages]);
  const childStates = JSON.stringify(tree.top.map((row) => [row.id, row.state]));
  const previousChildStates = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!observed) return;
    const previous = previousChildStates.current;
    previousChildStates.current = childStates;
    if (previous !== undefined && previous !== childStates) binding?.refreshProgress?.();
  }, [binding, childStates, observed]);
  const reload = useCallback(async () => {
    try {
      rememberPages(transport, space, (await transport.spaceHome()).pages);
    } catch {
      // The list stays as it was; the next read of it shows the change.
    }
  }, [transport, space]);
  const owner = binding?.work !== undefined && binding.canWork?.() === true;
  const [adding, setAdding] = useState(false);
  const stateControl = <PageStateMenu binding={binding} state={band.pageState} />;
  const shared = {
    ...band,
    stateControl,
    subpages: tree.top,
    childrenOf: tree.childrenOf,
    counts: observed ? tree.counts : undefined,
  };
  if (!owner) return <WorkBand {...shared} />;
  const add = async (choice: AddChoice): Promise<string | undefined> => {
    const added = await binding.work!('task.add', {
      task: { title: choice.kind === 'create' ? choice.title : choice.title.slice(0, 120) },
    }).catch((failure: unknown) => ({ failure }));
    if ('failure' in added)
      return explained(added.failure) ? saveMessage(added.failure) : t.attachUnknown;
    const taskId = added.taskId;
    if (taskId === undefined) return t.attachUnknown;
    try {
      if (choice.kind === 'create') await binding.work!('task.convert', { taskId });
      else await binding.work!('task.link', { taskId, pageId: choice.pageId });
      void reload();
      return undefined;
    } catch (failure) {
      if (!explained(failure)) return t.attachUnknown;
      // The panel shows no tasks, so the task made only to hold the link goes with its refusal.
      await binding.work!('task.remove', { taskId }).catch(() => undefined);
      const made = (failure as SaveRefused | SaveNotApplied).childPage;
      if (made !== undefined) {
        void reload();
        return t.madeNotLinked(choice.title);
      }
      return saveMessage(failure);
    }
  };
  const detach = async (row: SubpageRow) => {
    const remove = (on: PageBinding) => on.work!('task.remove', { taskId: row.carrier });
    if (row.parent === pageId) await remove(binding);
    else await onPage(transport, row.parent, remove);
    void reload();
  };
  return (
    <>
      <WorkBand
        {...shared}
        onAdd={() => setAdding(true)}
        rowState={(row) => (
          <RowStateMenu
            row={row}
            write={async (next) => {
              await onPage(transport, row.id, (on, shown) => writeState(on, shown, next));
              await reload();
            }}
          />
        )}
        rowActions={(row) => (
          <RowMenu row={row} href={band.pageHref?.(row.id)} detach={() => detach(row)} />
        )}
      />
      {adding && (
        <AddSubpageDialog
          title={title}
          pages={pages}
          refusal={tree.refusal}
          onClose={() => setAdding(false)}
          onChoose={add}
        />
      )}
    </>
  );
}

function RowMenu({
  row,
  href,
  detach,
}: {
  row: SubpageRow;
  href?: string;
  detach(): Promise<void>;
}) {
  const [error, setError] = useState<string>();
  return (
    <>
      <BrowserMenu
        label={t.more(row.title ?? t.unknownPage)}
        icon={<MoreHorizontal aria-hidden="true" />}
        items={[
          ...(href ? [{ key: 'open', label: t.open }] : []),
          { key: 'detach', label: t.detach },
        ]}
        onSelect={(key) => {
          if (key === 'open' && href) window.location.assign(href);
          if (key === 'detach')
            detach().then(
              () => setError(undefined),
              (failure: unknown) => setError(saveMessage(failure)),
            );
        }}
      />
      {error && <span role="alert">{error}</span>}
    </>
  );
}

function RowStateMenu({ row, write }: { row: SubpageRow; write(next: string): Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const label = row.state ? t.taskState[row.state] : t.unknown;
  return (
    <div className="work-state-control">
      <BrowserMenu
        label={t.rowState(row.title ?? t.unknownPage)}
        variant="count"
        valueLabel={label}
        disabled={busy}
        items={PAGE_STATES.map((value) => ({
          key: value,
          label: t.taskState[value],
          role: 'menuitemradio' as const,
          checked: value === row.state,
        }))}
        onSelect={(next) => {
          if (busy || next === row.state) return;
          setBusy(true);
          setError(undefined);
          write(next).then(
            () => setBusy(false),
            (failure: unknown) => {
              setBusy(false);
              setError(saveMessage(failure));
            },
          );
        }}
      />
      {error && <span role="alert">{error}</span>}
    </div>
  );
}

/** The verified label moves only when the owner's admitted stream publishes the commit. */
function PageStateMenu({ binding, state }: { binding?: PageBinding; state?: PageState | null }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const revision = useRef<Promise<string | undefined>>(undefined);
  const label = state ? t.taskState[state] : t.stateUnavailable;
  if (!binding?.canMetadata?.() || !binding.metadata || !binding.metadataRevision || state == null)
    return <span className={`work-page-state ${ui.countChip}`}>{label}</span>;
  const select = async (next: string) => {
    if (busy || !(PAGE_STATES as readonly string[]).includes(next)) return;
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
        label={t.pageState}
        variant="count"
        valueLabel={label}
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
        items={PAGE_STATES.map((value) => ({
          key: value,
          label: t.taskState[value],
          role: 'menuitemradio' as const,
          checked: value === state,
        }))}
        onSelect={(key) => {
          void select(key);
        }}
      />
      {busy && <span role="status">{t.savingState}</span>}
      {error && <span role="alert">{error}</span>}
    </div>
  );
}
