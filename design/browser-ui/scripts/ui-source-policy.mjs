/** Validate compiler output before publishing it inside the UI island. */
export function assertUiUtilities(css) {
  // CSS class/variable identifiers may end in body (for example .tmt\:font-body).
  // Reject the element selector, including compounds, rather than that identifier suffix.
  if (
    /@(?:property|import|theme|tailwind|layer)\b|:root|(?<![\w\\.#:-])(?:html|body)(?=[\s,[.#:>{])/.test(
      css,
    )
  )
    throw new Error('Unexpected global output from Tailwind; review generator before publishing');
}

/** Complete literal utilities only; Tailwind owns interpretation, never a second utility implementation. */
export function uiCandidates(source) {
  const literals = new Set();
  for (const match of source.matchAll(/\btmt:/g)) {
    let end = match.index;
    let brackets = 0;
    let quote = null;
    for (; end < source.length; end++) {
      const character = source[end];
      if (/\s/.test(character)) break;
      if (brackets) {
        if (quote) {
          if (character === quote && source[end - 1] !== '\\') quote = null;
        } else if (character === '"' || character === "'") quote = character;
        else if (character === '[') brackets++;
        else if (character === ']') brackets--;
      } else if (character === '[') brackets++;
      else if ('\'"`'.includes(character)) break;
    }
    const candidate = source.slice(match.index, end);
    if (brackets || quote || candidate === 'tmt:' || candidate.includes('${'))
      throw new Error('UI utilities require complete literal names and balanced selectors');
    literals.add(candidate);
  }
  const candidates = [...literals].sort();
  for (const candidate of candidates) {
    if (
      /(?:^|:)(?:bg|text|border|outline|fill|stroke)-[[(]/.test(candidate) &&
      !candidate.includes('--tmt-foundation-')
    )
      throw new Error('Arbitrary UI styling must reference the foundation token namespace');
    if (candidate.includes('border-radius:') && !candidate.includes('--tmt-foundation-radius'))
      throw new Error('UI radius must reference a foundation radius token');
    // A radius is a named token (`rounded-control`, `-small`, `-surface`, `-pill`); a shadow is a level
    // of the one elevation scale (#5821): `shadow-elevation-1`, `shadow-overlay` (level 2, floating
    // layers) or `shadow-elevation-3` (modal). A ring stays rejected: focus is a token outline.
    if (/(?:^|:)rounded(?:-(?!(?:control|small|surface|pill)$)|$)/.test(candidate))
      throw new Error(
        'UI radius must be a named token: rounded-control, -small, -surface or -pill',
      );
    if (/(?:^|:)shadow(?:-(?!(?:overlay|elevation-1|elevation-3)$)|$)/.test(candidate))
      throw new Error(
        'UI shadows are elevation levels: shadow-elevation-1, -overlay or -elevation-3',
      );
    // Motion reads the motion tokens only (#5821): no literal duration, delay or easing.
    if (
      /(?:^|:)(?:duration|delay)-(?!\(--tmt-foundation-motion-)/.test(candidate) ||
      /(?:^|:)ease-(?!\(--tmt-foundation-motion-)/.test(candidate) ||
      /\d(?:ms|s)(?![a-z])/.test(candidate)
    )
      throw new Error('UI motion must read the motion tokens: no literal duration or easing');
    if (/(?:^|:)ring(?:-|$)/.test(candidate))
      throw new Error('UI focus is a token-backed outline, never a ring');
    if (
      /(?:#|\b(?:rgb|hsl|oklch|oklab|lab|lch)\()|(?:^|:)(?:bg|text|border|outline|fill|stroke)-(?:black|white|red|blue|gray|slate|zinc|neutral|stone|orange|yellow|green|emerald|teal|cyan|sky|indigo|violet|purple|fuchsia|pink|rose)(?:-|$)/.test(
        candidate,
      )
    )
      throw new Error('UI colors must come from named browser token roles');
  }
  for (const match of source.matchAll(/\bclassName\s*=\s*["']([^"']*)["']/g))
    if (match[1].split(/\s+/).some((name) => name && !name.startsWith('tmt:')))
      throw new Error('UI utility classes require the fixed tmt: prefix');
  return candidates;
}
