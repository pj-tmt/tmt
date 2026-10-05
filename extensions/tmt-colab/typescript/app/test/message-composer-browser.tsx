/** Baseline overlay geometry. Fixture controls never prepare or dispatch a real Ask. */
import 'virtual:tokens.css';
import '../src/style.css';
import { mount } from './annotation-browser.js';

mount('multi');
const popover = document.getElementById('annotation-fixture')!;
popover.className = 'annotation-popover';
popover.setAttribute('role', 'dialog');
popover.setAttribute('aria-label', 'Annotation baseline');
popover.style.left = '12px';
popover.style.top = '80px';
// The reused fixture host is <main>; production SelectionAnnotation uses <div>.
popover.style.minHeight = '0';
const toolbar = document.createElement('nav');
toolbar.setAttribute('aria-label', 'Fixture position');
for (const position of ['Top', 'Bottom'] as const) {
  const button = document.createElement('button');
  button.textContent = position;
  button.addEventListener('click', () => {
    popover.style.top = position === 'Top' ? '80px' : 'auto';
    popover.style.bottom = position === 'Bottom' ? '12px' : 'auto';
  });
  toolbar.append(button);
}
document.body.prepend(toolbar);
