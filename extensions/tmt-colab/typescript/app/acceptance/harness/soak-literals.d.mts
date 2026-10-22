import policy from './soak-policy.json' with { type: 'json' };
export { policy };
export interface SoakDictionary {
  codes: Set<string>;
  messages: Set<string>;
  assertions: Record<string, string>;
}
export function loadSoakDictionary(read?: (file: string) => string): SoakDictionary;
