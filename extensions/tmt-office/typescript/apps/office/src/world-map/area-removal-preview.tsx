import { useMemo } from 'react';
import type { AreaPopulation } from '../local/office-population.js';
import { indexFloor } from './floor-index.js';
import { mapGeometry } from './map-source.js';
import { objectArea } from './object-area.js';
import type { WorldDocument } from './world-contract.js';
import type { previewModuleRemoval } from './module-authoring.js';

/** Explain the draft operation without performing or duplicating native validation. */
export function AreaRemovalPreview({
  world,
  population,
  removal,
}: {
  world: WorldDocument;
  population: AreaPopulation;
  removal?: ReturnType<typeof previewModuleRemoval>;
}) {
  const { area, members, room } = population;
  const objects = useMemo(() => {
    if (removal) return removal.blockedObjects;
    const floor = indexFloor(mapGeometry(world.map).floor);
    return world.objects.filter((object) => objectArea(floor, object) === area.id);
  }, [world, area.id, removal]);
  return (
    <section aria-label="Area removal preview">
      <h4>
        Remove {area.name}
        {removal ? ' module' : ' designation'}
      </h4>
      <p>
        {removal
          ? 'This room and its unused connecting floor leave the map. Identities, notes and messages are kept.'
          : 'Floor becomes common space. Objects, identities, notes and messages are kept.'}
      </p>
      <p>
        {area.binding.type === 'meeting'
          ? `Room ${room?.name ?? area.binding.roomId} and its membership are kept; only this space is unlinked.`
          : area.id === world.map.primaryLobbyId
            ? removal
              ? 'The central Lobby stays on the map.'
              : 'Occupants move to the replacement Lobby.'
            : 'Occupants return to the primary Lobby.'}
      </p>
      <details>
        <summary>
          {members.length}{' '}
          {area.binding.type === 'meeting' ? 'room members retained' : 'occupants affected'}
        </summary>
        <ul>
          {members.map((member) => (
            <li key={member.identityId}>{member.profile?.identityName ?? member.identityId}</li>
          ))}
        </ul>
      </details>
      <details>
        <summary>
          {objects.length} {removal ? 'placements require attention' : 'objects kept in place'}
        </summary>
        <ul>
          {objects.map((object) => (
            <li key={object.id}>
              {object.placement.prop.split('/').at(-1)} · {object.id}
            </li>
          ))}
        </ul>
      </details>
      {removal?.reason && <p role="status">{removal.reason}</p>}
      <p>
        {removal
          ? 'Removal applies automatically after confirmation. Required access paths must remain valid. No placement or linked content is removed automatically.'
          : 'Changes apply automatically. Invalid wall placements must be resolved; nothing is removed automatically.'}
      </p>
    </section>
  );
}
