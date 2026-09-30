import { expect, it, vi } from 'vitest';
vi.mock('./capture.js', () => ({
  captureTab: vi.fn(async () => ({ selection: 'chosen', url: 'https://x', title: 'title' })),
}));
vi.mock('./journal.js', () => ({ saveCapture: vi.fn(async () => {}) }));
it('registers only browser-installed/menu-click callbacks; a matching top-frame gesture captures without sending', async () => {
  const installed = vi.fn(),
    clicked = vi.fn();
  const api = {
    runtime: { onInstalled: { addListener: installed } },
    contextMenus: { create: vi.fn(), onClicked: { addListener: clicked } },
    action: { openPopup: vi.fn(async () => {}), setBadgeText: vi.fn(), setTitle: vi.fn() },
  };
  vi.stubGlobal('chrome', api);
  await import('./background.js');
  const { captureTab } = await import('./capture.js');
  const { saveCapture } = await import('./journal.js');
  installed.mock.calls[0][0]();
  expect(api.contextMenus.create).toHaveBeenCalledWith(
    expect.objectContaining({ contexts: ['selection'] }),
  );
  const handler = clicked.mock.calls[0][0];
  handler({ menuItemId: 'other' }, { id: 1 });
  handler({ menuItemId: 'tmt-send-selection', frameId: 3 }, { id: 1 });
  expect(captureTab).not.toHaveBeenCalled();
  handler({ menuItemId: 'tmt-send-selection', frameId: 0 }, { id: 7 });
  await vi.waitFor(() => expect(api.action.openPopup).toHaveBeenCalledTimes(1));
  expect(captureTab).toHaveBeenCalledWith(7);
  expect(saveCapture).toHaveBeenCalledWith({
    selection: 'chosen',
    url: 'https://x',
    title: 'title',
  });
  vi.unstubAllGlobals();
});
