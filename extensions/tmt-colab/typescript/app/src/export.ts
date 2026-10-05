import { decimal, digest, generatedId, requireValue, spaceId, text, time } from '@tmt/colab-client';
import {
  projectConversations,
  renderConversationsMarkdown,
  serializeConversations,
} from './conversations.js';
import { validateOwn, validateProjection, type OwnState } from './fold-protocol.js';

export const DISCLOSURE =
  'This creates an unencrypted copy of the page. Anyone with these files can read it.';
export type ExportFile = 'page.html' | 'conversations.json' | 'conversations.md' | 'manifest.json';
export const EXPORT_FILES: readonly ExportFile[] = [
  'page.html',
  'conversations.json',
  'conversations.md',
  'manifest.json',
];
/** One bundle never exceeds this many bytes of conversations (both files together). */
export const CONVERSATIONS_BYTES = 8 * 1024 * 1024;
export interface ExportView {
  spaceId: string;
  pageId: string;
  source: string;
  title: string;
  originalAuthor?: string;
  publisherAgent?: string;
  exportedAtMs: number;
  membershipHead: { revision: string; statementHash: string };
  epoch: string;
  /** The admitted per-writer discussion state and each writer's historical signing key. */
  own: OwnState;
  signingKeys: Record<string, Uint8Array>;
}
export interface FileInfo {
  readonly name: ExportFile;
  readonly sizeBytes: number;
  readonly sha256: string;
}
export function hex(bytes: Uint8Array): string {
  return [...bytes].map((byte) => byte.toString(16).padStart(2, '0')).join('');
}

/** Caller admission supplies the view. Copy everything before the first await;
 * hashing cannot mix a later projection/head into this plaintext bundle. */
export async function prepareExport(input: ExportView): Promise<ExportBundle> {
  const view = { ...input, membershipHead: { ...input.membershipHead } };
  const own = structuredClone(input.own);
  const keys = new Map(
    Object.entries(input.signingKeys).map(([writer, key]) => [writer, key.slice()]),
  );
  spaceId(view.spaceId);
  generatedId(view.pageId);
  decimal(view.epoch);
  decimal(view.membershipHead.revision);
  time(view.exportedAtMs);
  requireValue(
    typeof view.membershipHead.statementHash === 'string' &&
      /^[0-9a-f]{64}$/.test(view.membershipHead.statementHash),
  );
  validateProjection(view);
  validateOwn(own);
  const conversations = await projectConversations({
    spaceId: view.spaceId,
    pageId: view.pageId,
    title: view.title,
    epoch: view.epoch,
    membershipHead: view.membershipHead,
    own,
    signingKey: (writer) => keys.get(writer)?.slice(),
  });
  const html = text(view.source);
  const json = text(serializeConversations(conversations));
  const markdown = text(renderConversationsMarkdown(conversations));
  if (json.length + markdown.length > CONVERSATIONS_BYTES) throw new Error('EXPORT_TOO_LARGE');
  const describe = async (name: ExportFile, bytes: Uint8Array): Promise<FileInfo> =>
    Object.freeze({ name, sizeBytes: bytes.length, sha256: hex(await digest(bytes)) });
  const files = [
    await describe('page.html', html),
    await describe('conversations.json', json),
    await describe('conversations.md', markdown),
  ];
  // Order and spelling mirror native Manifest; export-v1.json pins identical bytes.
  const manifest = text(
    JSON.stringify({
      format: 'tmt-colab-page-export',
      version: 1,
      spaceId: view.spaceId,
      pageId: view.pageId,
      title: view.title,
      ...(view.originalAuthor === undefined ? {} : { originalAuthor: view.originalAuthor }),
      ...(view.publisherAgent === undefined ? {} : { publisherAgent: view.publisherAgent }),
      exportedAtMs: view.exportedAtMs,
      membershipHead: {
        revision: view.membershipHead.revision,
        statementHash: view.membershipHead.statementHash,
      },
      epoch: view.epoch,
      plaintext: true,
      discussions: {
        included: true,
        scope: 'current-epoch',
        format: conversations.format,
        version: conversations.version,
      },
      files,
    }),
  );
  return new ExportBundle(
    new Map<ExportFile, Uint8Array>([
      ['page.html', html],
      ['conversations.json', json],
      ['conversations.md', markdown],
      ['manifest.json', manifest],
    ]),
    [...files, await describe('manifest.json', manifest)],
  );
}

export class ExportBundle {
  readonly files: readonly FileInfo[];
  #bytes: Map<ExportFile, Uint8Array>;
  constructor(bytes: Map<ExportFile, Uint8Array>, files: readonly FileInfo[]) {
    this.#bytes = new Map([...bytes].map(([name, value]) => [name, value.slice()]));
    this.files = Object.freeze(files.map((file) => Object.freeze({ ...file })));
  }
  blob(name: ExportFile): Blob {
    const bytes = this.#bytes.get(name);
    requireValue(bytes !== undefined);
    return new Blob([new Uint8Array(bytes)], { type: 'application/octet-stream' });
  }
}

/** Parent-only lifecycle. A request is not evidence of a saved file. */
export class Downloads {
  #urls = new Map<string, ReturnType<typeof setTimeout>>();
  #closed = false;
  constructor(readonly bundle: ExportBundle) {}
  request(name: ExportFile): void {
    requireValue(!this.#closed);
    const url = URL.createObjectURL(this.bundle.blob(name));
    const anchor = document.createElement('a');
    try {
      anchor.href = url;
      anchor.download = name;
      anchor.hidden = true;
      document.body.append(anchor);
      anchor.click();
      // Let the browser consume its download request before revoking the URL.
      this.#urls.set(
        url,
        setTimeout(() => this.#revoke(url), 1000),
      );
    } catch (error) {
      URL.revokeObjectURL(url);
      throw error;
    } finally {
      anchor.remove();
    }
  }
  #revoke(url: string) {
    clearTimeout(this.#urls.get(url));
    this.#urls.delete(url);
    URL.revokeObjectURL(url);
  }
  close() {
    this.#closed = true;
    for (const url of this.#urls.keys()) this.#revoke(url);
  }
}
