import { useEffect, useRef, useState } from 'react';
import type { attachment } from '@tmt/colab-client';
import { GENERIC_MEDIA_TYPE, RASTER_TYPES, previewType } from './attachment-file.js';
import type { AttachmentBinding, AttachmentReference } from './attachment-service.js';
import { BlobDownloads } from './export.js';

const dataUrl = (blob: Blob) =>
  new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });

export const isRaster = (d: attachment.AttachmentDescriptor) =>
  (RASTER_TYPES as readonly string[]).includes(d.mediaType);

/** Bytes open only on an explicit click, each through the admitted read of the exact
 * reference given. A preview is a bounded raster shown from a `data:` URL; every other
 * file is only ever an octet-stream download. One read runs at a time. Everything shown or
 * held belongs to the disclosure scope it was read under: when the epoch, the admission head
 * or the connection's standing changes, previews and failures are disposed, the download
 * owner is replaced and a read still in flight is dropped. */
export function useAttachmentOpener(binding: AttachmentBinding | undefined) {
  const scope = binding?.scope?.() ?? '';
  const current = useRef(scope);
  current.current = scope;
  const downloads = useRef<BlobDownloads | null>(null);
  useEffect(() => {
    const owner = new BlobDownloads();
    downloads.current = owner;
    return () => {
      owner.close();
      downloads.current = null;
    };
  }, [scope]);
  const [busy, setBusy] = useState<string>();
  const [failed, setFailed] = useState<Set<string>>(new Set());
  const [shown, setShown] = useState<Record<string, string>>({});
  const [held, setHeld] = useState(scope);
  if (held !== scope) {
    setHeld(scope);
    setBusy(undefined);
    setFailed(new Set());
    setShown({});
  }
  const run = async (
    reference: AttachmentReference,
    d: attachment.AttachmentDescriptor,
    use: (bytes: Uint8Array) => Promise<void>,
  ) => {
    if (busy || !binding) return;
    const started = current.current;
    setBusy(d.attachmentId);
    setFailed((previous) => {
      const next = new Set(previous);
      next.delete(d.attachmentId);
      return next;
    });
    let bytes: Uint8Array | undefined;
    try {
      bytes = await binding.open(reference, d);
      // A disclosure that changed while the read ran owns nothing of this result.
      if (current.current !== started) return;
      await use(bytes);
    } catch {
      if (current.current === started)
        setFailed((previous) => new Set(previous).add(d.attachmentId));
    } finally {
      bytes?.fill(0);
      if (current.current === started) setBusy(undefined);
    }
  };
  return {
    busy,
    failed,
    shown,
    preview: (reference: AttachmentReference, d: attachment.AttachmentDescriptor) =>
      run(reference, d, async (bytes) => {
        const type = previewType(d.mediaType, bytes),
          started = current.current;
        if (!type) throw new Error('Not a bounded raster');
        const url = await dataUrl(new Blob([new Uint8Array(bytes)], { type }));
        if (current.current === started)
          setShown((previous) => ({ ...previous, [d.attachmentId]: url }));
      }),
    download: (reference: AttachmentReference, d: attachment.AttachmentDescriptor) =>
      run(reference, d, async (bytes) =>
        downloads.current?.request(
          new Blob([new Uint8Array(bytes)], { type: GENERIC_MEDIA_TYPE }),
          d.filename,
        ),
      ),
    hide: (id: string) => setShown(({ [id]: _hidden, ...rest }) => rest),
  };
}
