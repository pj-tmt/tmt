import { decimal, requireValue } from '@tmt/colab-client';
import type { Admission } from './admission.js';
import { Catchup } from './catchup.js';
import { Fold } from './fold.js';
import { Frames } from './frames.js';
import { Objects } from './objects.js';
import type { PageView } from './transport.js';

export interface AttachmentSnapshot {
  admission: Admission;
  objects: Objects;
  projection: PageView;
  revision: string;
}
/** The current connection supplies a bounded, exact-epoch ciphertext stream.
 * Values are still untrusted: Frames/Admission/Objects and the Worker admit them.
 * The source owns cancellation on deadline, peer close and replacement. */
export interface AttachmentHistorySource {
  frames(epoch: string, deadline: number): AsyncIterable<string>;
}
function remaining(deadline: number) {
  requireValue(Number.isFinite(deadline) && performance.now() < deadline);
}
/** Detached historical fold. No update or old baseline can enter the live
 * Worker, and a changed live head/cut/key/caller prevents returning its result. */
export async function attachmentHistory(
  current: () => Promise<AttachmentSnapshot>,
  source: AttachmentHistorySource,
  epoch: string,
  sharing: string | readonly string[],
  deadline: number,
  worker?: Worker,
): Promise<AttachmentSnapshot> {
  remaining(deadline);
  const initial = await current(),
    a = initial.admission;
  requireValue(decimal(epoch) < decimal(a.epoch));
  a.validateRead(sharing, epoch);
  const root = a.readRoot(epoch),
    objects = new Objects(a, epoch),
    catchup = new Catchup(a, sharing, objects, epoch, true),
    fold = new Fold(worker);
  let fault: Error | undefined,
    complete = false;
  const frames = new Frames(
    a,
    (error) => {
      fault = error;
    },
    epoch,
  );
  try {
    for await (const raw of source.frames(epoch, deadline)) {
      remaining(deadline);
      requireValue(fault === undefined && !complete);
      const frame = frames.receive(raw);
      if (frame) complete = await catchup.admitValue(frame);
    }
    remaining(deadline);
    requireValue(fault === undefined && complete);
    const baseline = catchup.baseline;
    if (baseline) {
      const projection = await fold.run({
        type: 'baseline',
        update: baseline.update,
        title: baseline.title,
        sourceDigest: baseline.sourceDigest,
        commitment: baseline.commitment,
      });
      requireValue(projection.source === baseline.source && projection.title === baseline.title);
    }
    for (const update of catchup.checkpoints) {
      remaining(deadline);
      await fold.run({
        type: 'checkpoint',
        update: update.update,
        writer: update.namespace === 'own' ? update.writer : undefined,
      });
    }
    const projection = await fold.run({
      type: 'apply',
      updates: catchup.updates.filter((v) => v.namespace === 'content').map((v) => v.update),
      own: catchup.updates
        .filter((v) => v.namespace === 'own')
        .map(({ writer, update }) => ({ writer, update })),
    });
    const fresh = await current();
    remaining(deadline);
    requireValue(
      fresh.admission === a && fresh.revision === initial.revision && a.readRoot(epoch) === root,
    );
    a.validateRead(sharing, epoch);
    return {
      admission: a,
      objects,
      projection: structuredClone(projection),
      revision: await objects.revision(),
    };
  } finally {
    frames.close();
    catchup.close();
    fold.close();
  }
}
