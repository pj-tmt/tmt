import { decimal, digest, generatedId, requireValue, spaceId, text, time } from '@tmt/colab-client';
import { validateProjection } from './fold-protocol.js';

export const DISCLOSURE =
  'This creates an unencrypted copy of the page. Anyone with these files can read it.';
export type ExportFile = 'page.html' | 'manifest.json';
export interface ExportView {
  spaceId: string;
  pageId: string;
  source: string;
  title: string;
  exportedAtMs: number;
  membershipHead: { revision: string; statementHash: string };
  epoch: string;
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
  const html = text(view.source);
  const file = Object.freeze({
    name: 'page.html' as const,
    sizeBytes: html.length,
    sha256: hex(await digest(html)),
  });
  // Order and spelling mirror native Manifest; export-v1.json pins identical bytes.
  const manifest = text(
    JSON.stringify({
      format: 'tmt-colab-page-export',
      version: 1,
      spaceId: view.spaceId,
      pageId: view.pageId,
      title: view.title,
      exportedAtMs: view.exportedAtMs,
      membershipHead: {
        revision: view.membershipHead.revision,
        statementHash: view.membershipHead.statementHash,
      },
      epoch: view.epoch,
      plaintext: true,
      discussions: 'not-included',
      files: [file],
    }),
  );
  return new ExportBundle(html, manifest, [
    file,
    Object.freeze({
      name: 'manifest.json',
      sizeBytes: manifest.length,
      sha256: hex(await digest(manifest)),
    }),
  ]);
}

export class ExportBundle {
  readonly files: readonly FileInfo[];
  #html: Uint8Array;
  #manifest: Uint8Array;
  constructor(html: Uint8Array, manifest: Uint8Array, files: readonly FileInfo[]) {
    this.#html = html.slice();
    this.#manifest = manifest.slice();
    this.files = Object.freeze(files.map((file) => Object.freeze({ ...file })));
  }
  blob(name: ExportFile): Blob {
    return new Blob([new Uint8Array(name === 'page.html' ? this.#html : this.#manifest)], {
      type: 'application/octet-stream',
    });
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
