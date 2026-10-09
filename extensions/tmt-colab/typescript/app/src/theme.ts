export type Theme = 'light' | 'dark';

/** The root stores only an explicit choice; otherwise the OS remains the default. */
export function getTheme(): Theme {
  const choice = document.documentElement.dataset.theme;
  return choice === 'light' || choice === 'dark'
    ? choice
    : matchMedia('(prefers-color-scheme: dark)').matches
      ? 'dark'
      : 'light';
}

export function subscribeTheme(changed: () => void): () => void {
  const preference = matchMedia('(prefers-color-scheme: dark)');
  const observer = new MutationObserver(changed);
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme'],
  });
  preference.addEventListener('change', changed);
  return () => {
    observer.disconnect();
    preference.removeEventListener('change', changed);
  };
}
