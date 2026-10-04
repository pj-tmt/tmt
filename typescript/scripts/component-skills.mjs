import { readFileSync } from 'node:fs';

const COMPONENT_MAP = new URL('../../.github/components.json', import.meta.url);

/**
 * Whether a native product's archive carries an agent-skills tree: the `skills` field of its
 * component in .github/components.json, the one declaration the archive policy, the verifier
 * and the tooling tests read. It needs only Node, so the verification-only image can run it.
 * A product matches the component named for it, with or without the `tmt-` prefix.
 */
export function shipsSkills(product, text = readFileSync(COMPONENT_MAP, 'utf8')) {
  const matches = Object.entries(JSON.parse(text).components ?? {}).filter(
    ([name]) => name === product || name === `tmt-${product}`
  );
  if (matches.length !== 1) throw new Error(`Ambiguous or missing component for ${product}.`);
  return matches[0][1].skills === true;
}
