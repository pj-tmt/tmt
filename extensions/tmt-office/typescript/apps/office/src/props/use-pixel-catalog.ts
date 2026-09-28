import { useCallback, useEffect, useRef, useState } from 'react';
import { LocalHttpError } from '../local/local-runtime.js';
import type { CatalogPack, PropPack } from './prop-contract.js';
import type {
  PropCatalogPage,
  PropCatalogPort,
  PropInstallInput,
} from './prop-catalog-contract.js';

type Save =
  | { kind: 'idle' }
  | { kind: 'pending' | 'uncertain'; input: PropInstallInput; pack: PropPack }
  | { kind: 'conflict' | 'rejected'; message: string }
  | { kind: 'saved'; catalog: CatalogPack };

/** Catalog observations and an exact frozen mutation; never another art store. */
export function usePixelCatalog(port: PropCatalogPort) {
  const [page, setPage] = useState<PropCatalogPage>();
  const [readError, setReadError] = useState<string>();
  const [reading, setReading] = useState(true);
  const [save, setSave] = useState<Save>({ kind: 'idle' });
  const read = useRef<AbortController | undefined>(undefined);
  const write = useRef<AbortController | undefined>(undefined);
  const readPage = useCallback(
    async (cursor?: string) => {
      read.current?.abort();
      const controller = new AbortController();
      read.current = controller;
      setReading(true);
      setReadError(undefined);
      try {
        const next = await port.list(cursor, controller.signal);
        if (controller.signal.aborted) return;
        setPage(next);
        return next;
      } catch {
        if (!controller.signal.aborted) {
          setPage(undefined);
          setReadError('Catalog could not load. Refresh the library; your drawing is kept.');
        }
      } finally {
        if (read.current === controller) read.current = undefined;
        if (!controller.signal.aborted) setReading(false);
      }
    },
    [port]
  );
  useEffect(() => {
    void readPage();
    return () => {
      read.current?.abort();
      write.current?.abort();
      read.current = undefined;
      write.current = undefined;
    };
  }, [readPage]);

  async function refresh(cursor?: string) {
    if (write.current || save.kind === 'uncertain') return;
    const next = await readPage(cursor);
    // A confirmed conflict is resolved only by this explicit catalog read.
    if (next && save.kind === 'conflict' && !cursor) setSave({ kind: 'idle' });
  }

  async function install(pack?: PropPack) {
    if (read.current || write.current || save.kind === 'conflict') return;
    const intent =
      save.kind === 'uncertain'
        ? save
        : pack && page
          ? { input: { expectedRevision: page.revision, document: JSON.stringify(pack) }, pack }
          : undefined;
    if (!intent) return;
    const controller = new AbortController();
    write.current = controller;
    setSave({ ...intent, kind: 'pending' });
    try {
      const receipt = await port.install(intent.input, controller.signal);
      if (controller.signal.aborted) return;
      setSave({ kind: 'saved', catalog: { digest: receipt.digest, pack: intent.pack } });
      // Existing cursors refer to the previous revision. Require a fresh page.
      setPage(undefined);
    } catch (cause) {
      if (controller.signal.aborted) return;
      if (cause instanceof LocalHttpError && cause.code === 'OFFICE_CATALOG_REVISION_CONFLICT') {
        setSave({
          kind: 'conflict',
          message: 'The catalog changed. Refresh the library, then review and save again.',
        });
      } else if (
        cause instanceof LocalHttpError &&
        [400, 401, 403, 409, 413, 422].includes(cause.status)
      ) {
        setSave({ kind: 'rejected', message: `${cause.message} Your drawing is kept.` });
      } else {
        // A missing/invalid receipt is not evidence that the write did not happen.
        setSave({ ...intent, kind: 'uncertain' });
      }
    } finally {
      if (write.current === controller) write.current = undefined;
    }
  }
  return {
    page,
    readError,
    reading,
    save,
    refresh,
    install,
    locked: save.kind === 'pending' || save.kind === 'uncertain',
    edited() {
      if (!write.current && save.kind !== 'uncertain' && save.kind !== 'conflict')
        setSave({ kind: 'idle' });
    },
    release() {
      if (write.current || save.kind !== 'uncertain') return;
      setSave({ kind: 'idle' });
      setPage(undefined);
      setReadError(
        'Frozen save released. It may already be stored. Refresh the library before saving again.'
      );
    },
  };
}
