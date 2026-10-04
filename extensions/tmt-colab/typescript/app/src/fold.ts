import { exactKeys, generatedId, requireValue } from '@tmt/colab-client';
import {
  BASELINE_UPDATE_BYTES,
  READ_TAIL_UPDATES,
  UPDATE_BYTES,
  STATE_BYTES,
  WRITE_TAIL_BYTES,
  WRITE_TAIL_UPDATES,
  validateProjection,
  validateOwn,
  type FoldCommand,
  type FoldResult,
} from './fold-protocol.js';

/** Decoder output is checked again by the authority-holding parent. Failure
 * terminates the Worker and flags the binding; callers must resync from scratch. */
export class Fold {
  #worker: Worker;
  #pending: {
    id: number;
    resolve(value: FoldResult): void;
    reject(error: Error): void;
    writers: Set<string>;
    commit: boolean;
  } | null = null;
  #writers = new Set<string>();
  #next = 0;
  #closed = false;
  #timer: ReturnType<typeof setTimeout> | undefined;
  constructor(
    worker = new Worker(new URL('./fold.worker.ts', import.meta.url), { type: 'module' }),
  ) {
    this.#worker = worker;
    worker.onerror = () => this.close();
    worker.onmessage = (event: MessageEvent<unknown>) => {
      try {
        exactKeys(event.data, [
          'id',
          'source',
          'title',
          'update',
          'own',
          ...(Object.hasOwn(event.data as object, 'publisherAgent') ? ['publisherAgent'] : []),
        ]);
        const value = event.data as unknown as FoldResult & { id: number; error?: string };
        if (!this.#pending || value.id !== this.#pending.id || value.error)
          throw new Error('Rejected decoder output');
        validateProjection(value);
        validateOwn(value.own);
        requireValue(
          Object.keys(value.own).length === this.#pending.writers.size &&
            Object.keys(value.own).every((writer) => this.#pending!.writers.has(writer)),
        );
        if (
          new TextEncoder().encode(
            JSON.stringify({
              source: value.source,
              title: value.title,
              publisherAgent: value.publisherAgent,
              own: value.own,
            }),
          ).length > STATE_BYTES
        )
          throw new Error('Decoder projection capacity');
        if (!(value.update instanceof Uint8Array) || value.update.length > UPDATE_BYTES)
          throw new Error('Invalid decoder output');
        const pending = this.#pending;
        if (pending.commit) this.#writers = pending.writers;
        this.#pending = null;
        clearTimeout(this.#timer);
        pending.resolve({
          source: value.source,
          title: value.title,
          ...(value.publisherAgent === undefined ? {} : { publisherAgent: value.publisherAgent }),
          own: value.own,
          update: value.update,
        });
      } catch {
        this.close();
      }
    };
  }
  run(command: FoldCommand): Promise<FoldResult> {
    if (this.#closed || this.#pending) return Promise.reject(new Error('Decoder unavailable'));
    if (
      (command.type === 'apply' || command.type === 'check') &&
      (command.updates.length + (command.own?.length ?? 0) >
        (command.type === 'apply' ? READ_TAIL_UPDATES : WRITE_TAIL_UPDATES) ||
        command.updates.reduce((n, v) => n + v.length, 0) +
          (command.own ?? []).reduce((n, v) => n + v.update.length, 0) >
          (command.type === 'apply' ? STATE_BYTES : WRITE_TAIL_BYTES))
    )
      return Promise.reject(new Error('Decoder input capacity'));
    if (command.type === 'checkpoint' && command.update.length > STATE_BYTES)
      return Promise.reject(new Error('Decoder checkpoint capacity'));
    if (
      command.type === 'baseline' &&
      (command.update.length > BASELINE_UPDATE_BYTES ||
        command.sourceDigest.length !== 32 ||
        command.commitment.length !== 32)
    )
      return Promise.reject(new Error('Decoder baseline capacity'));
    const writers = new Set(this.#writers);
    for (const writer of command.type === 'prepare-own'
      ? [command.writer]
      : command.type === 'checkpoint'
        ? command.writer === undefined
          ? []
          : [command.writer]
        : command.type === 'apply' || command.type === 'check'
          ? (command.own ?? []).map((v) => v.writer)
          : []) {
      generatedId(writer);
      writers.add(writer);
    }
    if (writers.size > 256) return Promise.reject(new Error('Decoder writer capacity'));
    return new Promise((resolve, reject) => {
      const id = ++this.#next;
      this.#pending = {
        id,
        resolve,
        reject,
        writers,
        commit:
          command.type !== 'check' && command.type !== 'prepare' && command.type !== 'prepare-own',
      };
      this.#timer = setTimeout(() => this.close(), 2000);
      try {
        this.#worker.postMessage({ id, command });
      } catch {
        this.close();
      }
    });
  }
  close() {
    this.#closed = true;
    clearTimeout(this.#timer);
    this.#worker.terminate();
    this.#pending?.reject(new Error('Decoder failed or exceeded its time budget'));
    this.#pending = null;
  }
}
