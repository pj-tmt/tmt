import { useState } from 'react';
import type { PropDefinition, PropPack } from '../props/prop-contract.js';
import { validPropCustomization } from '../props/prop-customization.js';
import { PropCustomizationFields } from '../props/prop-customization-fields.js';
import type { WorldObject } from './world-contract.js';

export function WorldObjectAppearance({
  object,
  pack,
  definition,
  change,
}: {
  object: WorldObject;
  pack: PropPack;
  definition: PropDefinition;
  change: (object: WorldObject) => void;
}) {
  const [value, setValue] = useState(object.placement.customization);
  const [error, setError] = useState(false);
  return (
    <form
      aria-label="Object appearance"
      onSubmit={(event) => {
        event.preventDefault();
        if (value && !validPropCustomization(value)) {
          setError(true);
          return;
        }
        const placement = { ...object.placement };
        if (value) placement.customization = value;
        else delete placement.customization;
        change({ ...object, placement });
        setError(false);
      }}
    >
      <PropCustomizationFields
        pack={pack}
        definition={definition}
        value={value}
        change={setValue}
      />
      <button type="submit">Apply appearance</button>
      {error && (
        <p role="alert">
          Use nonblank text up to 24 characters and 64 UTF-8 bytes, without control characters.
        </p>
      )}
    </form>
  );
}
