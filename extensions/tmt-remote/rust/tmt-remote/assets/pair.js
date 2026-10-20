// Capture and erase the secret before loading any other script.
(() => {
  const start = () => {
    const link = location.href;
    document.documentElement.dataset.tmtAuto = String(location.hash.startsWith('#auto='));
    history.replaceState(null, '', location.pathname);
    import('/sdk/remote-v1.js').then(({ pairingPage }) => pairingPage(link));
  };
  // A native opener can reuse a tab whose document is already /remote/pair.
  addEventListener('hashchange', (event) => {
    event.stopImmediatePropagation();
    start();
  });
  start();
})();
