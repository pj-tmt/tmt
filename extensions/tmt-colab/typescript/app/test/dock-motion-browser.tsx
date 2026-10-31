/** The dock's motion hook alone, on stand-in panels, to prove when it plays (#5192). */
import { flushSync } from 'react-dom';
import { createRoot, type Root } from 'react-dom/client';
import { MOTION, useDockMotion, type DockPanel } from '../src/dock-motion.js';

let root: Root | undefined;
const started: (string | null)[] = [];
const animate = Element.prototype.animate;

function Harness({ open }: { open: DockPanel | null }) {
  useDockMotion(open);
  return null;
}

/** Mounts the hook with `open` as its first value. */
export function mount(open: DockPanel | null) {
  root?.unmount();
  document.getElementById('dock-motion-fixture')?.remove();
  started.length = 0;
  Element.prototype.animate = function (this: Element, frames, options) {
    if ((options as KeyframeAnimationOptions | undefined)?.id === MOTION)
      started.push(this.getAttribute('data-panel'));
    return animate.call(this, frames, options);
  };
  const host = document.createElement('div');
  host.id = 'dock-motion-fixture';
  host.innerHTML = `
    <div class="page-floating-actions" style="position:fixed;right:16px;bottom:16px;width:96px;height:44px"></div>
    <div class="tmt-ui-panel" data-panel="chat" style="position:fixed;top:0;right:0;width:320px;height:400px;--tmt-ui-motion-base:180ms;--tmt-ui-motion-ease-out:ease-out;--tmt-ui-motion-ease-in:ease-in"></div>`;
  document.body.append(host);
  const mountPoint = document.createElement('div');
  host.append(mountPoint);
  root = createRoot(mountPoint);
  flushSync(() => root!.render(<Harness open={open} />));
}

/** Renders the hook again with another value. */
export function show(open: DockPanel | null) {
  flushSync(() => root!.render(<Harness open={open} />));
}

/** The panel of each motion started since `mount`. */
export const recorded = () => [...started];
