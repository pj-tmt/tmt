import { ExtensionEntry } from './extension-entry.js';
import type { WorldExtensionGroup } from './world-extension-groups.js';
import './world-object-actions.css';

interface Props {
  groups: WorldExtensionGroup[];
  areaId?: string;
  activate(id: string): void;
  focus(id?: string): void;
}

/** Area-first access with an explicit route to shared and other-area objects. */
export function WorldObjectActions({ groups, areaId, activate, focus }: Props) {
  const local = groups.find((group) => group.areaId === areaId);
  const common = groups.find((group) => group.areaId === null);
  const elsewhere = groups.filter((group) => group !== local && group !== common);
  function section(group: WorldExtensionGroup) {
    return (
      <section key={group.areaId ?? group.label} aria-label={`Tools in ${group.label}`}>
        <h3>{group.label}</h3>
        {group.entries.map((entry) => (
          <ExtensionEntry
            key={entry.instance.id}
            entry={entry}
            description={`Tile ${entry.instance.x}, ${entry.instance.y}`}
            activate={activate}
            focus={focus}
          />
        ))}
      </section>
    );
  }
  return (
    <nav className="inspector-object-actions" aria-label="Office objects">
      {local ? section(local) : <p>No interactive objects in this area.</p>}
      {common && common !== local && section(common)}
      {elsewhere.length > 0 && (
        <details>
          <summary>Other areas · {elsewhere.length}</summary>
          {elsewhere.map(section)}
        </details>
      )}
    </nav>
  );
}
