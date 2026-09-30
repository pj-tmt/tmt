import { isCapture } from './message.js';
import type { Capture } from './message.js';
export async function captureTab(tabId: number): Promise<Capture> {
  const results = await chrome.scripting.executeScript({
    target: { tabId, frameIds: [0] },
    world: 'ISOLATED',
    func: () => {
      const selection = window.getSelection()?.toString() ?? '';
      if (selection.length + location.href.length + document.title.length > 65536)
        throw new Error('Selection is too large.');
      return { selection, url: location.href, title: document.title };
    },
  });
  const capture: unknown = results[0]?.result;
  if (!isCapture(capture)) throw new Error('The selection is unavailable or too large.');
  if (!capture.selection) throw new Error('Select some text on the page first.');
  if (!/^https?:\/\//.test(capture.url))
    throw new Error('Capture is limited to HTTP and HTTPS pages.');
  return capture;
}
export async function captureActiveTab(): Promise<Capture> {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (tab?.id === undefined) throw new Error('No active page is available.');
  return captureTab(tab.id);
}
