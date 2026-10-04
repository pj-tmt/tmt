import { exactKeys, requireValue, strictJson, text } from '@tmt/colab-client';
import { Admission } from './admission.js';
import { Objects } from './objects.js';
import { openBaseline, type BaselineInput, type BaselineObject } from './baseline.js';
import { STATE_BYTES, UPDATE_BYTES, type AdmittedUpdate } from './fold-protocol.js';

/** Strict membership-first catchup. Optional content adoption stays behind object admission. */
export class Catchup {
  #started = false;
  #membershipMore = false;
  #complete = false;
  updates: AdmittedUpdate[] = [];
  checkpoints: AdmittedUpdate[] = [];
  baseline: BaselineInput | null = null;
  #reset: { descriptor: string; object: BaselineObject } | null = null;
  constructor(
    readonly admission: Admission,
    readonly sharing: string | readonly string[],
    readonly objects?: Objects,
  ) {}
  close() {
    this.checkpoints.forEach((v) => v.update.fill(0));
    this.checkpoints = [];
    this.updates.forEach((v) => v.update.fill(0));
    this.updates = [];
    this.baseline?.update.fill(0);
    this.baseline = null;
    this.#reset = null;
  }
  async admit(raw: string): Promise<boolean> {
    return this.admitValue(strictJson(text(raw), 64 * 1024, true));
  }
  async admitValue(v: unknown): Promise<boolean> {
    const a = this.admission;
    const keys = ['version', 'type', 'space', 'page', 'epoch', 'streams', 'more'];
    requireValue(v !== null && typeof v === 'object' && !Array.isArray(v));
    const value = v as Record<string, unknown>;
    if (value.type === 'error') {
      exactKeys(value, ['version', 'type', 'space', 'page', 'epoch', 'code']);
      this.#scope(value);
      requireValue(
        typeof value.code === 'string' &&
          [
            'DENIED',
            'EXPIRED',
            'STALE_EPOCH',
            'INVALID',
            'GAP',
            'CAPACITY',
            'CONFLICT',
            'RESYNC_REQUIRED',
          ].includes(value.code),
      );
      throw new Error(`Sync bootstrap rejected: ${value.code}`);
    }
    requireValue(!this.#complete && value.type === 'catchup');
    for (const key of [
      'membershipHead',
      'baseline',
      'baselineObject',
      'membership',
      'chains',
      'wraps',
    ])
      if (Object.hasOwn(value, key)) keys.push(key);
    exactKeys(value, keys);
    this.#scope(value);
    requireValue(
      Array.isArray(value.streams) &&
        value.streams.length <= 256 &&
        typeof value.more === 'boolean',
    );
    if (!this.#started) {
      requireValue(
        Object.hasOwn(value, 'membershipHead') &&
          Object.hasOwn(value, 'baseline') &&
          !Object.hasOwn(value, 'membership') &&
          value.streams.length === 0 &&
          value.more,
      );
      await a.membership(value.membershipHead, true);
      this.#membershipMore = (value.membershipHead as { more: boolean }).more;
      if (value.baseline !== null) {
        requireValue(typeof value.baseline === 'string' && this.objects !== undefined);
        exactKeys(value.baselineObject, ['envelopeHash', 'envelope']);
        requireValue(
          typeof value.baselineObject.envelope === 'string' &&
            typeof value.baselineObject.envelopeHash === 'string',
        );
        this.#reset = {
          descriptor: value.baseline,
          object: value.baselineObject as unknown as BaselineObject,
        };
      } else {
        requireValue(!Object.hasOwn(value, 'baselineObject') && a.epoch === '1');
      }
      this.#started = true;
    } else {
      requireValue(
        !Object.hasOwn(value, 'membershipHead') &&
          !Object.hasOwn(value, 'baseline') &&
          !Object.hasOwn(value, 'baselineObject'),
      );
      if (this.#membershipMore) {
        requireValue(
          Object.hasOwn(value, 'membership') && value.streams.length === 0 && value.more,
        );
        await a.membership(value.membership, false);
        this.#membershipMore = (value.membership as { more: boolean }).more;
      } else requireValue(!Object.hasOwn(value, 'membership'));
    }
    if (Object.hasOwn(value, 'chains') || Object.hasOwn(value, 'wraps'))
      requireValue(!this.#membershipMore);
    if (Object.hasOwn(value, 'chains')) await a.chains(value.chains);
    if (Object.hasOwn(value, 'wraps')) await a.wraps(value.wraps);
    if (value.streams.length) {
      if (!this.objects) throw new Error('Live page loading is not available yet');
      const admitted = await this.objects.streams(value.streams);
      this.checkpoints.push(...admitted.checkpoints);
      this.updates.push(...admitted.updates);
      requireValue(
        this.checkpoints.length <= 256 &&
          this.checkpoints.reduce((n, v) => n + v.update.length, 0) <= STATE_BYTES,
      );
      requireValue(
        this.updates.length <= 200 &&
          this.updates.reduce((n, v) => n + v.update.length, 0) <= UPDATE_BYTES,
      );
    }
    if (!value.more) {
      requireValue(!this.#membershipMore && a.head !== null);
      this.objects?.finish();
      a.validatePage(this.sharing);
      if (this.#reset) {
        this.baseline = await openBaseline(a, this.#reset.descriptor, this.#reset.object);
        this.#reset = null;
      }
      this.#complete = true;
    }
    return this.#complete;
  }
  #scope(value: Record<string, unknown>) {
    requireValue(
      value.version === 1 &&
        value.space === this.admission.space &&
        value.page === this.admission.page &&
        value.epoch === this.admission.epoch,
    );
  }
}
