import { useEffect, useRef, useState } from 'react';
import type { attachment } from '@tmt/colab-client';
import { GENERIC_MEDIA_TYPE, RASTER_TYPES } from './attachment-file.js';
import { rasterDataUrl } from './attachment-raster.js';
import type { AttachmentBinding, AttachmentReference } from './attachment-service.js';
import { readFailure, type ReadFailure } from './attachments.js';
import { BlobDownloads } from './export.js';

export const isRaster = (d: attachment.AttachmentDescriptor) =>
  (RASTER_TYPES as readonly string[]).includes(d.mediaType);

/** One serial admitted read owner. Message thumbnails may enter when visible; Files and
 * downloads enter only on click. Display copies belong to this binding, disclosure scope
 * and exact reference lifetime, and disappear on replacement or unmount. */
export function useAttachmentOpener(binding: AttachmentBinding | undefined, referenceKey = '') {
  const scope = binding?.scope?.() ?? '';
  const context = useRef({
    binding,
    scope,
    referenceKey,
    generation: 0,
    alive: true,
    tail: Promise.resolve(),
  });
  if (
    context.current.binding !== binding ||
    context.current.scope !== scope ||
    context.current.referenceKey !== referenceKey
  ) {
    context.current.alive = false;
    context.current = {
      binding,
      scope,
      referenceKey,
      generation: context.current.generation + 1,
      alive: true,
      tail: context.current.tail,
    };
  }
  const owner = context.current;
  const downloads = useRef<BlobDownloads | null>(null);
  const [state, setState] = useState({
    owner,
    busy: undefined as string | undefined,
    failed: new Map<string, ReadFailure>(),
    shown: {} as Record<string, string>,
  });
  if (state.owner !== owner) setState({ owner, busy: undefined, failed: new Map(), shown: {} });
  const cache = useRef({
    owner,
    urls: new Map<string, string>(),
    pending: new Map<string, { work: Promise<void>; visible?: () => boolean }>(),
  });
  if (cache.current.owner !== owner) {
    cache.current.urls.clear();
    cache.current = { owner, urls: new Map(), pending: new Map() };
  }
  useEffect(() => {
    owner.alive = true;
    const downloadOwner = new BlobDownloads();
    downloads.current = downloadOwner;
    return () => {
      owner.alive = false;
      if (cache.current.owner === owner) cache.current.urls.clear();
      downloadOwner.close();
      downloads.current = null;
    };
  }, [owner]);
  const current = () =>
    owner.alive && context.current === owner && (binding?.scope?.() ?? '') === scope;
  const run = (
    reference: AttachmentReference,
    d: attachment.AttachmentDescriptor,
    use: (bytes: Uint8Array) => Promise<void>,
    permitted: () => boolean = () => true,
  ) => {
    const work = owner.tail.then(async () => {
      if (!binding || !current() || !permitted()) return;
      setState((previous) => ({
        ...previous,
        busy: d.attachmentId,
        failed: new Map([...previous.failed].filter(([id]) => id !== d.attachmentId)),
      }));
      let bytes: Uint8Array | undefined;
      try {
        bytes = await binding.open(reference, d);
        if (current()) await use(bytes);
      } catch (error) {
        if (current())
          setState((previous) => ({
            ...previous,
            failed: new Map(previous.failed).set(d.attachmentId, readFailure(error)),
          }));
      } finally {
        bytes?.fill(0);
        if (current()) setState((previous) => ({ ...previous, busy: undefined }));
      }
    });
    owner.tail = work;
    return work;
  };
  const preview = (
    reference: AttachmentReference,
    d: attachment.AttachmentDescriptor,
    visible?: () => boolean,
  ) => {
    const held = cache.current;
    if (held.urls.has(d.attachmentId)) {
      const url = held.urls.get(d.attachmentId)!;
      if (current())
        setState((previous) => ({
          ...previous,
          shown: { ...previous.shown, [d.attachmentId]: url },
        }));
      return Promise.resolve();
    }
    const pending = held.pending.get(d.attachmentId);
    if (pending) {
      // Explicit viewer selection upgrades an automatic read that is still queued.
      if (!visible) pending.visible = undefined;
      return pending.work;
    }
    const request = { work: Promise.resolve(), visible };
    const work = run(
      reference,
      d,
      async (bytes) => {
        const url = await rasterDataUrl(d.mediaType, bytes);
        if (current()) {
          held.urls.set(d.attachmentId, url);
          setState((previous) => ({
            ...previous,
            shown: { ...previous.shown, [d.attachmentId]: url },
          }));
        }
      },
      () => request.visible?.() ?? true,
    ).finally(() => held.pending.delete(d.attachmentId));
    request.work = work;
    held.pending.set(d.attachmentId, request);
    return work;
  };
  return {
    lifetime: owner,
    busy: state.owner === owner ? state.busy : undefined,
    failed: state.owner === owner ? state.failed : new Map<string, ReadFailure>(),
    shown: state.owner === owner ? state.shown : {},
    preview,
    thumbnail: (
      reference: AttachmentReference,
      d: attachment.AttachmentDescriptor,
      visible: () => boolean,
    ) =>
      state.owner === owner && state.failed.has(d.attachmentId)
        ? Promise.resolve()
        : preview(reference, d, visible),
    download: (reference: AttachmentReference, d: attachment.AttachmentDescriptor) =>
      run(reference, d, async (bytes) => {
        if (current())
          downloads.current?.request(
            new Blob([new Uint8Array(bytes)], { type: GENERIC_MEDIA_TYPE }),
            d.filename,
          );
      }),
    hide: (id: string) => {
      cache.current.urls.delete(id);
      setState(({ shown, ...rest }) => {
        const { [id]: _hidden, ...remaining } = shown;
        return { ...rest, shown: remaining };
      });
    },
  };
}
