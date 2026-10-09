import type { FrozenAttachmentUpload } from './attachment-channel.js';
import {
  attachmentFilename,
  refuseFile,
  storedMediaType,
  type FileRefusal,
} from './attachment-file.js';
import {
  AttachmentUploadError,
  type AttachmentBinding,
  type AttachmentTarget,
  type RefusalReason,
  type StoredAttachment,
} from './attachment-service.js';
import type { MessageAttachments } from './thread-store.js';

export type ChipState =
  | { kind: 'ready' }
  | { kind: 'uploading'; sent: number; total: number }
  | { kind: 'stored'; stored: StoredAttachment }
  /** Not usable as it is; the local bytes upload again under a fresh original. */
  | { kind: 'again'; why: 'page-changed' | 'not-stored'; old?: FrozenAttachmentUpload }
  /** Possibly stored. Only status of this same original, or removal, resolves it. */
  | { kind: 'unknown'; original: FrozenAttachmentUpload }
  | { kind: 'refused'; reason: RefusalReason };

export interface Chip {
  id: string;
  filename: string;
  mediaType: string;
  size: number;
  state: ChipState;
}
export interface Notice {
  id: string;
  filename: string;
  reason: FileRefusal | 'unreadable';
}
export interface DraftSnapshot {
  chips: readonly Chip[];
  notices: readonly Notice[];
  messageId: string;
}
export type PrepareResult =
  | { ok: true; attach: MessageAttachments | undefined }
  | { ok: false; why: 'unknown' | 'refused' | 'failed' };

/** One composer's attachments. Picking, pasting and dropping only add local chips:
 * nothing is stored, sent or prepared until `prepare` runs inside an explicit Send, and
 * the message text is never read or changed here. Local bytes stay with the chip so a
 * stale or lost upload attaches again without asking for the file a second time. */
export class AttachmentDraft {
  #chips: (Chip & { bytes: Uint8Array })[] = [];
  #notices: Notice[] = [];
  #messageId = crypto.randomUUID();
  #listeners = new Set<() => void>();
  #snapshot: DraftSnapshot;
  /** The binding is read at each use: it may appear or be replaced while the composer lives. */
  constructor(
    private source: () => AttachmentBinding | undefined,
    /** Where uploads belong at the moment they run; a message draft uses its own ID. */
    private target?: () => AttachmentTarget,
  ) {
    this.#snapshot = this.#view();
  }
  get binding(): AttachmentBinding {
    const binding = this.source();
    if (!binding) throw new Error('Attachments are unavailable');
    return binding;
  }
  subscribe = (listener: () => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };
  getSnapshot = () => this.#snapshot;
  #view(): DraftSnapshot {
    return {
      chips: this.#chips.map(({ bytes: _bytes, ...chip }) => ({ ...chip })),
      notices: [...this.#notices],
      messageId: this.#messageId,
    };
  }
  #emit() {
    this.#snapshot = this.#view();
    for (const listener of this.#listeners) listener();
  }
  #set(id: string, state: ChipState) {
    const chip = this.#chips.find((c) => c.id === id);
    if (chip) chip.state = state;
    this.#emit();
  }
  get uploading() {
    return this.#chips.some((c) => c.state.kind === 'uploading');
  }
  async add(files: readonly File[]) {
    for (const file of files) {
      const id = crypto.randomUUID(),
        filename = attachmentFilename(file.name),
        refusal = refuseFile(file.size, this.#chips.length);
      if (refusal) {
        this.#notices.push({ id, filename, reason: refusal });
        this.#emit();
        continue;
      }
      try {
        const bytes = new Uint8Array(await file.arrayBuffer());
        // The file may have changed since it was picked; the bytes read are the truth.
        const again = refuseFile(bytes.length, this.#chips.length);
        if (again) throw new Error(again);
        this.#chips.push({
          id,
          filename,
          mediaType: storedMediaType(bytes),
          size: bytes.length,
          bytes,
          state: { kind: 'ready' },
        });
      } catch {
        this.#notices.push({ id, filename, reason: 'unreadable' });
      }
      this.#emit();
    }
  }
  dismiss(id: string) {
    this.#notices = this.#notices.filter((n) => n.id !== id);
    this.#emit();
  }
  /** Removes the chip; whatever it left in storage is released best effort. */
  remove(id: string) {
    const chip = this.#chips.find((c) => c.id === id);
    if (!chip || chip.state.kind === 'uploading') return;
    this.#chips = this.#chips.filter((c) => c !== chip);
    this.#release(chip.state);
    this.#emit();
  }
  #release(state: ChipState) {
    const original =
      state.kind === 'stored'
        ? state.stored.original
        : state.kind === 'unknown'
          ? state.original
          : state.kind === 'again'
            ? state.old
            : undefined;
    if (original) void this.source()?.discard(original);
  }
  /** An explicit second try after a refusal. Nothing was stored, so the bytes simply go again. */
  retry(id: string) {
    const chip = this.#chips.find((c) => c.id === id);
    if (chip?.state.kind === 'refused') this.#set(id, { kind: 'ready' });
  }
  /** Asks status of the same original; a pending transfer continues, never replays. */
  async check(id: string) {
    const chip = this.#chips.find((c) => c.id === id);
    if (!chip || chip.state.kind !== 'unknown') return;
    const { original } = chip.state;
    this.#set(id, { kind: 'uploading', sent: 0, total: 1 });
    try {
      const stored = await this.binding.resume(
        original,
        { filename: chip.filename, size: chip.size },
        (sent, total) => this.#set(id, { kind: 'uploading', sent, total }),
      );
      this.#set(id, { kind: 'stored', stored });
    } catch (error) {
      this.#set(id, failed(error, original));
    }
  }
  /** Uploads what still needs it, in order, stopping at the first failure. */
  async prepare(): Promise<PrepareResult> {
    if (this.#chips.some((c) => c.state.kind === 'unknown')) return { ok: false, why: 'unknown' };
    if (this.#chips.some((c) => c.state.kind === 'refused')) return { ok: false, why: 'refused' };
    for (const chip of this.#chips) {
      if (chip.state.kind !== 'ready' && chip.state.kind !== 'again') continue;
      const old = chip.state.kind === 'again' ? chip.state.old : undefined;
      this.#set(chip.id, { kind: 'uploading', sent: 0, total: 1 });
      try {
        const stored = await this.binding.upload(
          { filename: chip.filename, mediaType: chip.mediaType, bytes: chip.bytes },
          this.target?.() ?? { kind: 'message', messageId: this.#messageId },
          (sent, total) => this.#set(chip.id, { kind: 'uploading', sent, total }),
        );
        if (old) void this.binding.discard(old);
        this.#set(chip.id, { kind: 'stored', stored });
      } catch (error) {
        this.#set(chip.id, failed(error));
        return { ok: false, why: 'failed' };
      }
    }
    const stored = this.#chips.flatMap((c) => (c.state.kind === 'stored' ? [c.state.stored] : []));
    return {
      ok: true,
      attach: stored.length ? { messageId: this.#messageId, stored } : undefined,
    };
  }
  /** The page moved after these originals froze their base. */
  stale(ids: readonly string[]) {
    for (const chip of this.#chips)
      if (
        chip.state.kind === 'stored' &&
        ids.includes(chip.state.stored.original.descriptor.attachmentId)
      )
        chip.state = { kind: 'again', why: 'page-changed', old: chip.state.stored.original };
    this.#emit();
  }
  /** The message and its references were published: the chips are done, not released. */
  committed() {
    this.#chips = [];
    this.#notices = [];
    this.#messageId = crypto.randomUUID();
    this.#emit();
  }
  /** Leaving the composer unsent releases what it stored; local bytes simply drop. */
  dispose() {
    for (const chip of this.#chips) this.#release(chip.state);
    this.#chips = [];
  }
}

function failed(error: unknown, original?: FrozenAttachmentUpload): ChipState {
  if (error instanceof AttachmentUploadError) {
    const { failure } = error;
    if (failure.kind === 'refused') return { kind: 'refused', reason: failure.reason };
    if (failure.kind === 'gone') return { kind: 'again', why: 'not-stored' };
    return { kind: 'unknown', original: failure.original };
  }
  // Anything else after a status question leaves the answer unknown; before one it never started.
  return original ? { kind: 'unknown', original } : { kind: 'refused', reason: 'unavailable' };
}
