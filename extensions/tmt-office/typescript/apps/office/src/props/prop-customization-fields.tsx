import { useId } from 'react';
import type { PropPack, PropDefinition } from './prop-contract.js';
import type { PropCustomization } from './prop-customization.js';

/** Shared controls for admitted channels; the enclosing editor owns commit timing. */
export function PropCustomizationFields({
  pack,
  definition,
  value,
  change,
}: {
  pack: PropPack;
  definition: PropDefinition;
  value?: PropCustomization;
  change: (value: PropCustomization | undefined) => void;
}) {
  const guidance = useId();
  const capabilities = definition.customization;
  function edit(field: 'tint' | 'text', input: string) {
    const next = { ...value };
    if (input) next[field] = input;
    else delete next[field];
    change(Object.keys(next).length ? next : undefined);
  }
  return (
    <>
      {capabilities?.tint && (
        <>
          <label>
            Tint color
            <input
              type="color"
              value={value?.tint ?? pack.palette[capabilities.tint.indices[0]!]!.slice(0, 7)}
              onChange={(event) => edit('tint', event.target.value)}
            />
          </label>
          <button type="button" disabled={!value?.tint} onClick={() => edit('tint', '')}>
            Restore original color
          </button>
        </>
      )}
      {capabilities?.text && (
        <>
          <label>
            Display text
            <input
              type="text"
              value={value?.text ?? ''}
              aria-describedby={guidance}
              onChange={(event) => edit('text', event.target.value)}
            />
          </label>
          <small id={guidance}>
            Up to 24 characters and 64 UTF-8 bytes. Leave empty for no text.
          </small>
        </>
      )}
    </>
  );
}
