/** The TypeScript side of the cross-language work vectors (`contracts/vectors/work-v2.json`, Rust
 * `work.rs`): applying an action to a work value. Production never applies an action in the
 * browser (the serve prepares it natively), so this lives with the tests that need it. Mirrors
 * `work.rs`. */
import { id, keys, object, requireValue } from '../src/closed.js';
import { setSummary, type Attribution } from '../src/summary-lines.js';
import { migratedWork, validateWork, type Work } from '../src/work.js';

/** A page that has had no work yet carries no `meta.work`; its first action starts from this. */
export function emptyWork(): Work {
  return { version: 2, order: [] };
}
/** Produces a detached version 2 result or refuses the whole action; the base is not mutated.
 * `attribution` comes from the write owner, never from the action body. */
export function applyWorkAction(
  base: unknown,
  action: string,
  input: unknown,
  attribution: Attribution,
): Work {
  validateWork(base);
  const next = structuredClone(migratedWork(base));
  const body = object(input);
  if (action === 'summary.set') next.summary = setSummary(body, attribution);
  else if (action === 'summary.clear') {
    keys(body, []);
    delete next.summary;
  } else if (action === 'work.migrate') keys(body, []);
  else {
    requireValue(['subpage.attach', 'subpage.move', 'subpage.detach'].includes(action));
    keys(body, ['pageId'], action === 'subpage.detach' ? [] : ['before']);
    const page = body.pageId;
    id(page);
    const listed = next.order.indexOf(page);
    if (action === 'subpage.detach') {
      requireValue(listed >= 0);
      next.order.splice(listed, 1);
    } else {
      requireValue(listed >= 0 === (action === 'subpage.move'));
      if (listed >= 0) next.order.splice(listed, 1);
      let at = next.order.length;
      if (Object.hasOwn(body, 'before')) {
        const before = body.before;
        id(before);
        requireValue(before !== page);
        at = next.order.indexOf(before);
        requireValue(at >= 0);
      }
      next.order.splice(at, 0, page);
    }
  }
  validateWork(next);
  return next;
}
