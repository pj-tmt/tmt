import { AttachmentDraft } from './attachment-draft.js';
import type { AttachmentBinding } from './attachment-service.js';

/** The attachment drafts of one question group, one per question. The group owns them as it owns
 * its text drafts, so they survive disclosure changes and switching the open question; like the
 * text, they do not survive a reload. Adding a file only makes an inert local chip: nothing is
 * stored or sent until the group's Send. */
export class AnswerFiles {
  #drafts = new Map<string, AttachmentDraft>();
  #stops: (() => void)[] = [];
  #listeners = new Set<() => void>();
  #version = 0;
  /** The binding is read at each use: it may appear or be replaced while the group lives. */
  constructor(private source: () => AttachmentBinding | undefined) {}
  /** The question's draft, made when first asked for. */
  draft(question: string): AttachmentDraft {
    let draft = this.#drafts.get(question);
    if (!draft) {
      draft = new AttachmentDraft(this.source);
      this.#stops.push(draft.subscribe(() => this.#changed()));
      this.#drafts.set(question, draft);
    }
    return draft;
  }
  /** A file of this question is mid-transfer, so its answer waits and no other answer does. */
  uploading(question: string): boolean {
    return this.#drafts.get(question)?.uploading ?? false;
  }
  subscribe = (listener: () => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };
  getSnapshot = () => this.#version;
  #changed() {
    this.#version++;
    for (const listener of this.#listeners) listener();
  }
  /** Leaving the group unsent releases what its drafts stored; the local bytes just drop. */
  dispose() {
    for (const stop of this.#stops) stop();
    for (const draft of this.#drafts.values()) draft.dispose();
    this.#stops = [];
    this.#drafts.clear();
  }
}
