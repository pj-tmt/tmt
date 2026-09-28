import { StrictMode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { LocalHttpError } from '../local/local-runtime.js';
import { usePixelCatalog } from './use-pixel-catalog.js';
import { newPixelDraft, paintPixels, pixelDraftPack } from './pixel-draft.js';
import type {
  PropCatalogPage,
  PropInstallInput,
  PropInstallReceipt,
} from './prop-catalog-contract.js';

const pack = pixelDraftPack(paintPixels(newPixelDraft(), [{ x: 1, y: 2 }], 3));
const page: PropCatalogPage = { revision: 4, entries: [], excluded: [], nextCursor: null };
const receipt: PropInstallReceipt = {
  revision: 5,
  digest: `sha256:${'a'.repeat(64)}`,
  changed: true,
};
function portFixture() {
  return {
    list: vi.fn(async (_cursor?: string, _signal?: AbortSignal) => page),
    install: vi.fn(async (_input: PropInstallInput, _signal?: AbortSignal) => receipt),
    load: vi.fn(),
  };
}

it('loads under StrictMode, cancels superseded reads and never installs while reading', async () => {
  const port = portFixture();
  const view = renderHook(() => usePixelCatalog(port), { wrapper: StrictMode });
  await waitFor(() => expect(view.result.current.page).toEqual(page));
  expect(port.list).toHaveBeenCalledTimes(2);
  expect(port.list.mock.calls[0]![1]!.aborted).toBe(true);
  let finish!: (value: PropCatalogPage) => void;
  port.list.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      })
  );
  act(() => {
    void view.result.current.refresh();
  });
  await act(() => view.result.current.install(pack));
  expect(port.install).not.toHaveBeenCalled();
  view.unmount();
  expect(port.list.mock.calls[2]![1]!.aborted).toBe(true);
  await act(async () => finish({ ...page, revision: 20 }));
});

it('keeps exact bytes and revision after an unknown write result; Retry never resubmits edited art', async () => {
  const port = portFixture();
  port.install.mockRejectedValueOnce(new Error('connection dropped after storage commit'));
  const view = renderHook(() => usePixelCatalog(port));
  await waitFor(() => expect(view.result.current.page).toEqual(page));
  await act(() => view.result.current.install(pack));
  expect(view.result.current.save.kind).toBe('uncertain');
  expect(view.result.current.locked).toBe(true);
  act(() => view.result.current.edited());
  await act(() => view.result.current.refresh());
  expect(port.list).toHaveBeenCalledTimes(1);
  const input = structuredClone(port.install.mock.calls[0]![0]);
  port.install.mockResolvedValueOnce({ ...receipt, changed: false });
  await act(() => view.result.current.install({ ...pack, label: 'Must not replace frozen art' }));
  expect(port.install.mock.calls[1]![0]).toEqual(input);
  expect(input).toEqual({ expectedRevision: 4, document: JSON.stringify(pack) });
  expect(view.result.current.save).toEqual({
    kind: 'saved',
    catalog: { digest: receipt.digest, pack },
  });
  expect(view.result.current.page).toBeUndefined();
  expect(port.list).toHaveBeenCalledTimes(1);
});

it('requires explicit refresh after conflict and distinguishes capacity rejection from concurrency', async () => {
  const port = portFixture();
  const view = renderHook(() => usePixelCatalog(port));
  await waitFor(() => expect(view.result.current.page).toEqual(page));
  port.install.mockRejectedValueOnce(new LocalHttpError(409, 'OFFICE_CATALOG_REVISION_CONFLICT'));
  await act(() => view.result.current.install(pack));
  expect(view.result.current.save.kind).toBe('conflict');
  await act(() => view.result.current.install(pack));
  expect(port.install).toHaveBeenCalledTimes(1);
  port.list.mockResolvedValueOnce({ ...page, revision: 9 });
  await act(() => view.result.current.refresh());
  expect(view.result.current.save.kind).toBe('idle');
  expect(port.install).toHaveBeenCalledTimes(1);
  port.install.mockRejectedValueOnce(new LocalHttpError(409, 'OFFICE_PROP_LIMIT'));
  await act(() => view.result.current.install(pack));
  expect(port.install.mock.calls[1]![0].expectedRevision).toBe(9);
  expect(view.result.current.save).toMatchObject({
    kind: 'rejected',
    message: expect.stringContaining('OFFICE_PROP_LIMIT'),
  });
});

it('guards double submission synchronously and releasing uncertainty does not claim rollback', async () => {
  const port = portFixture();
  let reject!: (cause: unknown) => void;
  port.install.mockImplementationOnce(
    () =>
      new Promise((_resolve, fail) => {
        reject = fail;
      })
  );
  const view = renderHook(() => usePixelCatalog(port));
  await waitFor(() => expect(view.result.current.page).toEqual(page));
  act(() => {
    void view.result.current.install(pack);
    void view.result.current.install(pack);
  });
  expect(port.install).toHaveBeenCalledTimes(1);
  expect(view.result.current.save.kind).toBe('pending');
  act(() => view.result.current.release());
  expect(view.result.current.save.kind).toBe('pending');
  await act(async () => reject(new Error('unknown')));
  act(() => view.result.current.release());
  expect(view.result.current.save.kind).toBe('idle');
  expect(view.result.current.readError).toContain('may already be stored');
  expect(view.result.current.page).toBeUndefined();
  await act(() => view.result.current.install(pack));
  expect(port.install).toHaveBeenCalledTimes(1);
});
