import { registerBrowserElements } from '@tmt/browser-ui/elements';
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import htm from 'htm';
import { icons } from './icons.js';

/**
 * The bundled page runtime (#1693): what an opted-in page's own script finds on `window.tmt`, with
 * no network and no build step. `html` is htm bound to React, so a page writes components as tagged
 * templates instead of JSX. Nothing here talks to the parent; the frame stays an opaque sandbox.
 */
registerBrowserElements();

const runtime = Object.freeze({
  React,
  createRoot,
  html: htm.bind(React.createElement),
  icons,
});
Object.defineProperty(window, 'tmt', { value: runtime, enumerable: true });
