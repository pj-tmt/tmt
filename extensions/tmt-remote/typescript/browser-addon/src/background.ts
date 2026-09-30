import { captureTab } from './capture.js';
import { saveCapture } from './journal.js';
const menuId = 'tmt-send-selection';
chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: menuId,
    title: 'Send to agent',
    contexts: ['selection'],
    documentUrlPatterns: ['http://*/*', 'https://*/*'],
  });
});
chrome.contextMenus.onClicked.addListener((info, tab) => {
  if (info.menuItemId !== menuId || tab?.id === undefined || (info.frameId ?? 0) !== 0) return;
  // The browser event is the sole menu entry point; no page/message listener.
  void captureTab(tab.id)
    .then(saveCapture)
    .then(() => chrome.action.openPopup())
    .catch(() => {
      void chrome.action.setBadgeText({ text: '!' });
      void chrome.action.setTitle({
        title: 'Selection could not be captured. Open the popup and try again.',
      });
    });
});
