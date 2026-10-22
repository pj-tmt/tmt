import * as React from 'react';
import { createRoot } from 'react-dom/client';
import htm from 'htm';

/**
 * The bundled page runtime (#1693): what an opted-in page's own script finds on `window.tmt`, with
 * no network and no build step. `html` is htm bound to React, so a page writes components as tagged
 * templates instead of JSX. Nothing here talks to the parent; the frame stays an opaque sandbox.
 */
const runtime = Object.freeze({
  React,
  createRoot,
  html: htm.bind(React.createElement),
  /** The curated icon set joins in a later slice; an empty frozen namespace until then. */
  icons: Object.freeze({}),
});
Object.defineProperty(window, 'tmt', { value: runtime, enumerable: true });
