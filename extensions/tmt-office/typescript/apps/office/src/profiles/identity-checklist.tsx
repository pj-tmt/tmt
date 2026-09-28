import type { IdentityChoice } from './identity-choice.js';
import { presenceLabel, type Presence } from '../identities/presence.js';
import './identity-checklist.css';

interface Props {
  legend: string;
  choices: (IdentityChoice & { presence: Presence })[];
  selected: IdentityChoice[];
  limit: number;
  disabled?: boolean;
  onChange(selected: IdentityChoice[]): void;
}
/** Share selection and retained-missing controls between explicit requests and room editing. */
export function IdentityChecklist({ legend, choices, selected, limit, disabled, onChange }: Props) {
  const selectedIds = new Set(selected.map((item) => item.id));
  const visibleIds = new Set(choices.map((item) => item.id));
  const options = [
    ...selected
      .filter((item) => !visibleIds.has(item.id))
      .map((item) => ({ ...item, presence: 'unknown' as const })),
    ...choices,
  ];
  return (
    <fieldset className="identity-checklist" disabled={disabled}>
      <legend>{legend}</legend>
      {options.map((item) => (
        <label key={item.id}>
          <input
            type="checkbox"
            checked={selectedIds.has(item.id)}
            disabled={!selectedIds.has(item.id) && selectedIds.size >= limit}
            onChange={(event) =>
              onChange(
                event.target.checked
                  ? [...selected, { id: item.id, name: item.name }]
                  : selected.filter((choice) => choice.id !== item.id)
              )
            }
          />
          {item.name}
          {!visibleIds.has(item.id)
            ? ' · selected, not in current list'
            : item.presence === 'active'
              ? ''
              : ` · ${presenceLabel[item.presence].toLowerCase()}`}
        </label>
      ))}
    </fieldset>
  );
}
