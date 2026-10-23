import { shellIconNodes, type ShellIcon } from './shell-icons.js';
import { browserUiClasses } from './static.js';

const SVG = 'http://www.w3.org/2000/svg';

/** A line icon from the shared icon nodes, for a framework-free page: it takes the text colour. */
export function createIcon(name: ShellIcon, doc: Document = document): SVGSVGElement {
  const svg = doc.createElementNS(SVG, 'svg');
  svg.setAttribute('class', browserUiClasses.glyph);
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('aria-hidden', 'true');
  for (const [tag, attributes] of shellIconNodes[name]) {
    const node = doc.createElementNS(SVG, tag);
    for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
    svg.append(node);
  }
  return svg;
}
