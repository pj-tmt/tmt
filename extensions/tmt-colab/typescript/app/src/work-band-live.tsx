import type { ReactNode } from 'react';
import { WorkBand, type WorkBandProps } from './work-band.js';
import { AddTask, TaskControls, useWorkRun } from './work-controls.js';
import type { PageBinding } from './transport.js';

/** The page's band with the owner's controls, where this device may change the work; anywhere
 * else it is the read-only band. */
export function PageWorkBand({
  binding,
  ...band
}: WorkBandProps & { binding?: PageBinding }): ReactNode {
  const work = useWorkRun(binding);
  if (!binding?.work || !binding.canWork?.()) return <WorkBand {...band} />;
  return (
    <WorkBand
      {...band}
      actions={(task, id, state) => (
        <TaskControls key={id} id={id} task={task} state={state} work={work} />
      )}
      listActions={<AddTask work={work} />}
    />
  );
}
