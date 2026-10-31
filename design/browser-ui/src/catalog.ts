import evidence from '../generated/elements-catalog.json';
import { browserElements } from './elements-manifest.js';

/** Build-owned coverage describes indexed documentation and discovered tests, not CI pass state. */
export const browserCatalog = browserElements.map((entry) => {
  const coverage = evidence.entries.find((item) => item.tag === entry.tag);
  if (!coverage) throw new Error(`Missing catalog evidence: ${entry.tag}`);
  return { ...entry, tokens: coverage.tokens, coverage };
});
export const browserPresentationInventory = evidence.inventory;
