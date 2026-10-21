import type { ReactNode } from 'react';
import { WorkBand, type WorkBandProps } from './work-band.js';
import { AddTask, TaskControls, useWorkRun } from './work-controls.js';
import type { PageBinding, PageTransport } from './transport.js';
import { useLinkedStage } from './work-stages.js';

/** The page's band with the owner's controls, where this device may change the work; anywhere
 * else it is the read-only band. */
export function PageWorkBand({
  binding,
  transport,
  space,
  ...band
}: WorkBandProps & {
  binding?: PageBinding;
  /** Where a followed page's stage is read: the owner list this viewer already has. */
  transport: PageTransport;
  space?: string;
}): ReactNode {
  const work = useWorkRun(binding);
  const stageOf = useLinkedStage(transport, space, band.work);
  if (!binding?.work || !binding.canWork?.()) return <WorkBand {...band} stageOf={stageOf} />;
  return (
    <WorkBand
      {...band}
      stageOf={stageOf}
      actions={(task, id, state) => (
        <TaskControls key={id} id={id} task={task} state={state} work={work} />
      )}
      listActions={<AddTask work={work} />}
    />
  );
}
