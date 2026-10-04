// Capture and erase the secret before loading any other script.
(() => {
  const link = location.href;
  history.replaceState(null, '', location.pathname);
  import('/sdk/remote-v1.js').then(({ pairingPage }) => pairingPage(link));
})();
