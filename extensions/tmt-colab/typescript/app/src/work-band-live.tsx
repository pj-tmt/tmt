import type { ReactNode } from 'react';
import { WorkBand, type WorkBandProps } from './work-band.js';
import { AddTask, TaskControls, useWorkRun } from './work-controls.js';
import type { PageBinding, PageTransport } from './transport.js';
import { useLinkedPages } from './work-stages.js';
import type { Work } from './work.js';

/** A page that never had work, drawn only to name the page it belongs to. */
const NO_WORK: Work = { version: 1, next: 1, order: [], tasks: {} };

/** The page's band with the owner's controls, where this device may change the work; anywhere
 * else it is the read-only band. */
export function PageWorkBand({
  binding,
  transport,
  space,
  pageId,
  ...band
}: Omit<WorkBandProps, 'work' | 'parent'> & {
  /** Absent for a page that never had work: its band then shows only where it belongs. */
  work?: Work;
  binding?: PageBinding;
  /** Where a followed page's stage and this page's parent are read: the owner list this viewer
   * already has. */
  transport: PageTransport;
  space?: string;
  pageId: string;
}): ReactNode {
  const run = useWorkRun(binding);
  const { stageOf, parent } = useLinkedPages(transport, space, pageId, band.work);
  if (!band.work) {
    return parent ? <WorkBand {...band} work={NO_WORK} parent={parent} /> : null;
  }
  if (!binding?.work || !binding.canWork?.())
    return <WorkBand {...band} work={band.work} stageOf={stageOf} parent={parent} />;
  return (
    <WorkBand
      {...band}
      work={band.work}
      parent={parent}
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
