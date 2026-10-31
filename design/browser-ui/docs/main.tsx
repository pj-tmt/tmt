// Dev-only design-system docs and the Colab mockup (#5821), in palette B. See vite.docs.config.ts.
import '../generated/static.css';
import '../generated/ui.css';
import '../generated/editorial.css';
import './docs.css';
import './mock-colab.css';
import './mock-reader.css';
import { StrictMode, useEffect } from 'react';
import { createRoot } from 'react-dom/client';
import { DocsApp } from './app.js';
import { setPaletteBFavicon } from './favicon.js';
import { ColabMock } from './mock-colab.js';
import { usePath } from './router.js';

function Root() {
  const path = usePath();
  useEffect(() => {
    if (path === '/' || path === '') history.replaceState(null, '', '/docs');
  }, [path]);
  if (path.startsWith('/mock/colab')) return <ColabMock path={path} />;
  return <DocsApp path={path === '/' ? '/docs' : path} />;
}

setPaletteBFavicon();
createRoot(document.querySelector('#root')!).render(
  <StrictMode>
    <Root />
  </StrictMode>,
);
