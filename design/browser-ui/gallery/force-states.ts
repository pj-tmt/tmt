/**
 * Pinned hover and focus states from the leaf's real CSS. The components style these states only
 * with `:hover` and `:focus-visible`, which a page cannot hold on several elements at once. This
 * mirrors every loaded rule that uses them, swapping the pseudo-class for a gallery attribute, so a
 * cell can pin the state on its control. Nothing is copied by hand: an edit to the leaf's CSS
 * reaches the mirror on the next HMR update.
 */
export const forcedAttribute = {
  hover: 'data-gallery-hover',
  focus: 'data-gallery-focus',
} as const;
export type ForcedState = keyof typeof forcedAttribute;

const STYLE_ID = 'gallery-forced-states';

function mirror(rules: CSSRuleList, out: string[]): void {
  for (const rule of Array.from(rules)) {
    if (rule instanceof CSSStyleRule) {
      const selector = rule.selectorText;
      // Unescaped pseudo-classes only: utility class names such as `.tmt\:hover\:x` keep theirs.
      const forced = selector
        .replace(/(?<!\\):hover\b/g, `[${forcedAttribute.hover}]`)
        .replace(/(?<!\\):focus-visible\b/g, `[${forcedAttribute.focus}]`);
      if (forced !== selector) out.push(`${forced}{${rule.style.cssText}}`);
      // Nested rules (the UI layer nests its utilities under `:where([data-tmt-ui])`).
      const nested: string[] = [];
      mirror(rule.cssRules, nested);
      if (nested.length) out.push(`${selector}{${nested.join('')}}`);
    } else if (rule instanceof CSSMediaRule) {
      const inner: string[] = [];
      mirror(rule.cssRules, inner);
      if (inner.length) out.push(`@media ${rule.conditionText}{${inner.join('')}}`);
    } else if (rule instanceof CSSSupportsRule || rule instanceof CSSLayerBlockRule) {
      mirror(rule.cssRules, out);
    }
  }
}

/** Rebuild the mirror from every stylesheet the page has loaded. */
export function refreshForcedStates(document: Document): void {
  const out: string[] = [];
  for (const sheet of Array.from(document.styleSheets)) {
    if ((sheet.ownerNode as Element | null)?.id === STYLE_ID) continue;
    try {
      mirror(sheet.cssRules, out);
    } catch {
      // A cross-origin sheet (the web fonts) has no readable rules and no component states.
    }
  }
  let style = document.getElementById(STYLE_ID);
  if (!style) {
    style = document.createElement('style');
    style.id = STYLE_ID;
    document.head.append(style);
  }
  style.textContent = out.join('\n');
}

/** Pin `state` on the first interactive control inside `cell`. */
export function pinState(cell: HTMLElement, state: ForcedState): () => void {
  const control = cell.querySelector<HTMLElement>(
    'button, a[href], input, textarea, select, [tabindex], [role="tab"]',
  );
  control?.setAttribute(forcedAttribute[state], '');
  return () => control?.removeAttribute(forcedAttribute[state]);
}
