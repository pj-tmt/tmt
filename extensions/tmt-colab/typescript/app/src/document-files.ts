import type { attachment } from '@tmt/colab-client';
import type {
  AttachmentBinding,
  AttachmentService,
  AttachmentTarget,
  StoredAttachment,
} from './attachment-service.js';
import type { OwnRecord } from './fold-protocol.js';

/** The page's own files: uploads bound to the current source, published as an `intents` proof
 * first and then written into the document through Save, so a changed page never attaches. */
export interface DocumentFiles {
  /** Uploads and authorized reads over the page's object channel. */
  readonly attachments: AttachmentBinding;
  /** The upload target now: the exact source a new file is bound to. */
  target(): AttachmentTarget;
  /** Prove, then write, uploaded files into the page; a changed page is a stale error. */
  add(stored: readonly StoredAttachment[]): Promise<void>;
  /** Remove references; the stored bytes stay unreferenced. */
  remove(attachmentIds: readonly string[]): Promise<void>;
}

export interface DocumentFilesOptions {
  service: AttachmentService;
  /** The admitted source a save is based on. */
  source(): string;
  publish(records: OwnRecord[]): Promise<void>;
  edit(source: string, base: string, change: attachment.DocumentChange): Promise<void>;
}

export class LiveDocumentFiles implements DocumentFiles {
  constructor(private options: DocumentFilesOptions) {}
  get attachments() {
    return this.options.service;
  }
  target(): AttachmentTarget {
    return { kind: 'document', source: this.options.source() };
  }
  async add(stored: readonly StoredAttachment[]) {
    if (!stored.length) return;
    const source = this.options.source();
    await this.options.publish(await this.options.service.publication(stored, source));
    await this.options.edit(source, source, {
      set: stored.map(({ original }) => original.descriptor),
    });
  }
  async remove(attachmentIds: readonly string[]) {
    if (!attachmentIds.length) return;
    const source = this.options.source();
    await this.options.edit(source, source, { remove: [...attachmentIds] });
  }
}
