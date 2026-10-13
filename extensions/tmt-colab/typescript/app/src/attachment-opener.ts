import { useEffect, useRef, useState } from 'react';
import type { attachment } from '@tmt/colab-client';
import { GENERIC_MEDIA_TYPE, RASTER_TYPES } from './attachment-file.js';
import { rasterDataUrl } from './attachment-raster.js';
import type { AttachmentBinding, AttachmentReference } from './attachment-service.js';
import { readFailure, type ReadFailure } from './attachments.js';
import { BlobDownloads } from './export.js';
import { sortedJson } from './sorted-json.js';

export const isRaster = (d: attachment.AttachmentDescriptor) =>
  (RASTER_TYPES as readonly string[]).includes(d.mediaType);

/** One serial admitted read owner. Message thumbnails may enter when visible; Files and
 * downloads enter only on click. Display copies belong to this binding, disclosure scope
 * and exact reference lifetime, and disappear on replacement or unmount. */
export function useAttachmentOpener(
  binding: AttachmentBinding | undefined,
  reference: AttachmentReference,
  descriptors: readonly attachment.AttachmentDescriptor[],
) {
  // A list read again after a combine lists the same descriptors with their keys in another
  // order: the key is the value, not the order a map happens to hold it in.
  const referenceKey = sortedJson([reference, descriptors]);
  const inputs = useRef({ binding, reference, descriptors, referenceKey });
  inputs.current = { binding, reference, descriptors, referenceKey };
  const scope = binding?.scope?.() ?? '';
  const context = useRef({
    binding,
    scope,
    referenceKey,
    generation: 0,
    alive: true,
  });
  const resolveOwner = () => {
    const latest = inputs.current;
    const scope = latest.binding?.scope?.() ?? '';
    if (
      context.current.binding !== latest.binding ||
      context.current.scope !== scope ||
      context.current.referenceKey !== latest.referenceKey
    ) {
      context.current.alive = false;
      context.current = {
        binding: latest.binding,
        scope,
        referenceKey: latest.referenceKey,
        generation: context.current.generation + 1,
        alive: true,
      };
    }
    return context.current;
  };
  const owner = resolveOwner();
  const tail = useRef(Promise.resolve());
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
    const downloadOwner = new BlobDownloads();
    downloads.current = downloadOwner;
    return () => {
      context.current.alive = false;
      downloadOwner.close();
      downloads.current = null;
    };
  }, []);
  useEffect(() => {
    owner.alive = true;
    return () => {
      owner.alive = false;
      if (cache.current.owner === owner) cache.current.urls.clear();
    };
  }, [owner]);
  const current = (active = owner) =>
    active.alive &&
    context.current === active &&
    inputs.current.binding === active.binding &&
    inputs.current.referenceKey === active.referenceKey &&
    (active.binding?.scope?.() ?? '') === active.scope;
  const failure = (id: string, reason: ReadFailure) => {
    if (!downloads.current) return;
    const active = resolveOwner();
    setState((previous) => ({
      owner: active,
      busy: undefined,
      shown: previous.owner === active ? previous.shown : {},
      failed: new Map(previous.owner === active ? previous.failed : []).set(id, reason),
    }));
  };
  const run = (
    d: attachment.AttachmentDescriptor,
    use: (
      bytes: Uint8Array,
      descriptor: attachment.AttachmentDescriptor,
      active: typeof owner,
    ) => Promise<void>,
    automatic?: () => boolean | undefined,
  ) => {
    const work = tail.current.then(async () => {
      if (!downloads.current) return;
      const visible = automatic?.();
      if (visible === false) return;
      // Automatic thumbnails belong to the original display lifetime. A trusted click
      // instead selects the current listed ID once, before its admitted read starts.
      const explicit = visible === undefined;
      // A document file is fenced by the page revision. When a combine another device made is
      // still unseen here, the connection reopens first, which changes the scope and would end a
      // read that was under way; the owner is taken after.
      if (explicit && inputs.current.reference.kind === 'document') {
        try {
          await inputs.current.binding?.settle?.();
        } catch (error) {
          failure(d.attachmentId, readFailure(error));
          return;
        }
      }
      const active = explicit ? resolveOwner() : owner;
      const latest = inputs.current;
      const selected = explicit
        ? latest.descriptors.find((item) => item.attachmentId === d.attachmentId)
        : d;
      if (!active.binding || !selected || !current(active)) {
        if (explicit) failure(d.attachmentId, 'changed');
        return;
      }
      setState((previous) => ({
        owner: active,
        busy: selected.attachmentId,
        shown: previous.owner === active ? previous.shown : {},
        failed: new Map(
          [...(previous.owner === active ? previous.failed : [])].filter(
            ([id]) => id !== selected.attachmentId,
          ),
        ),
      }));
      let bytes: Uint8Array | undefined;
      try {
        bytes = await active.binding.open(explicit ? latest.reference : reference, selected);
        if (current(active)) await use(bytes, selected, active);
        if (!current(active) && explicit) failure(selected.attachmentId, 'changed');
      } catch (error) {
        if (current(active)) failure(selected.attachmentId, readFailure(error));
        else if (explicit) failure(selected.attachmentId, 'changed');
      } finally {
        bytes?.fill(0);
        if (current(active)) setState((previous) => ({ ...previous, busy: undefined }));
      }
    });
    tail.current = work;
    return work;
  };
  const preview = (d: attachment.AttachmentDescriptor, visible?: () => boolean) => {
    const held = cache.current;
    if (current() && held.urls.has(d.attachmentId)) {
      const url = held.urls.get(d.attachmentId)!;
      if (current())
        setState((previous) => ({
          ...previous,
          shown: { ...previous.shown, [d.attachmentId]: url },
        }));
      return Promise.resolve();
    }
    const pending = current() ? held.pending.get(d.attachmentId) : undefined;
    if (pending) {
      // Explicit viewer selection upgrades an automatic read that is still queued.
      if (!visible) pending.visible = undefined;
      return pending.work;
    }
    const request = { work: Promise.resolve(), visible };
    const work = run(
      d,
      async (bytes, selected, active) => {
        const url = await rasterDataUrl(selected.mediaType, bytes);
        if (current(active)) {
          const target =
            cache.current.owner === active
              ? cache.current
              : {
                  owner: active,
                  urls: new Map<string, string>(),
                  pending: new Map<string, { work: Promise<void>; visible?: () => boolean }>(),
                };
          cache.current = target;
          target.urls.set(selected.attachmentId, url);
          setState((previous) => ({
            ...previous,
            shown: { ...previous.shown, [d.attachmentId]: url },
          }));
        }
      },
      () => request.visible?.(),
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
    unlistedFailure:
      state.owner === owner &&
      [...state.failed.keys()].some((id) => !descriptors.some((d) => d.attachmentId === id)),
    preview,
    thumbnail: (d: attachment.AttachmentDescriptor, visible: () => boolean) =>
      state.owner === owner && state.failed.has(d.attachmentId)
        ? Promise.resolve()
        : preview(d, visible),
    download: (d: attachment.AttachmentDescriptor) =>
      run(d, async (bytes, selected, active) => {
        if (current(active))
          downloads.current?.request(
            new Blob([new Uint8Array(bytes)], { type: GENERIC_MEDIA_TYPE }),
            selected.filename,
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
