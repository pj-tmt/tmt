// Erase before the parser encounters any resource. Never report the private URL.
(() => {
  let dispose = () => {};
  let generation = 0;
  const start = () => {
    dispose();
    const current = ++generation;
    let token = location.hash.slice(1);
    try {
      history.replaceState(null, '', location.pathname);
      if (location.hash) throw new Error();
    } catch {
      token = '';
      return;
    }
    dispose = () => {
      token = '';
    };
    if (!document.querySelector('link[href="/sdk/invite.css"]')) {
      const style = document.createElement('link');
      style.rel = 'stylesheet';
      style.href = '/sdk/invite.css';
      document.head.append(style);
    }
    const load = () => {
      if (generation !== current) return;
      import('/sdk/invite-v1.js')
        .then(({ invitePage }) => {
          if (generation === current) dispose = invitePage(token);
          token = '';
        })
        .catch(() => {
          token = '';
        });
    };
    if (document.readyState === 'loading')
      document.addEventListener('DOMContentLoaded', load, { once: true });
    else load();
  };
  addEventListener('hashchange', (event) => {
    event.stopImmediatePropagation();
    start();
  });
  addEventListener('pagehide', () => {
    ++generation;
    dispose();
  });
  start();
})();
