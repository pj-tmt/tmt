/** Complete literal utilities only; Tailwind owns interpretation, never a second utility implementation. */
export function uiCandidates(source) {
  const candidates = [...new Set(source.match(/\btmt:[^\s'"`]+/g) ?? [])].sort();
  for (const candidate of candidates) {
    if (
      /(?:^|:)(?:bg|text|border|outline|fill|stroke)-[[(]/.test(candidate) &&
      !candidate.includes('--tmt-foundation-')
    )
      throw new Error('Arbitrary UI styling must reference the foundation token namespace');
    if (candidate.includes('border-radius:') && !candidate.includes('--tmt-foundation-radius'))
      throw new Error('UI radius must reference the square foundation token');
    if (/(?:^|:)(?:rounded|shadow|ring)(?:-|$)/.test(candidate))
      throw new Error('UI is square and shadow-free; use token-backed outline for focus');
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
