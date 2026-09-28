/** Trusted, single-line display text projected inside an admitted pixel rectangle. */
export interface InsetText {
  value: string;
  color: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

export const INSET_TEXT_FONT = 'monospace';

export function insetTextGeometry(text: InsetText) {
  const units = Array.from(text.value).reduce(
    (sum, character) => sum + (character.codePointAt(0)! < 128 ? 0.65 : 1.05),
    0
  );
  return {
    fontSize: Math.min(text.height * 0.72, text.width / Math.max(1, units)),
    x: text.x + text.width / 2,
    y: text.y + text.height / 2,
  };
}
