import { useState } from 'react';
import { presenceLabel } from '../identities/presence.js';
import type { OfficeSelection } from '../rendering/office-selection.js';
import type { AreaMember, AreaPopulation, OfficePopulation } from './office-population.js';

function matches(member: AreaMember, query: string) {
  return [
    member.identityId,
    member.profile?.identityName,
    member.profile?.profile.displayLabel,
  ].some((value) => value?.toLowerCase().includes(query));
}

function Member({
  member,
  select,
  areaId,
}: {
  member: AreaMember;
  select: (value: OfficeSelection) => void;
  areaId: string;
}) {
  const { identityId, profile } = member;
  if (!profile)
    return (
      <li>
        Unavailable identity <code>{identityId}</code>
      </li>
    );
  return (
    <li>
      <button onClick={() => select({ kind: 'agent', identityId, areaId })}>
        {profile.identityName} · {presenceLabel[profile.presence]}
        {profile.lifetime === 'temporary' && <small>Contractor</small>}
      </button>
    </li>
  );
}

export function AreaRoster({
  population,
  select,
}: {
  population: AreaPopulation;
  select: (value: OfficeSelection) => void;
}) {
  const [query, setQuery] = useState('');
  const filtered = population.members.filter((member) =>
    matches(member, query.trim().toLowerCase())
  );
  const { area, room, members } = population;
  if (area.binding.type === 'meeting' && !room)
    return (
      <section className="area-roster" aria-label="Area roster">
        <p role="status">The linked meeting room is unavailable. No membership is inferred.</p>
      </section>
    );
  return (
    <section className="area-roster" aria-label="Area roster">
      {room && (
        <p>
          Meeting: {room.name} · revision {room.revision}
        </p>
      )}
      <h3>
        {area.binding.type === 'meeting' ? 'Members' : 'Residents'} · {members.length}
      </h3>
      <p>
        The map previews a few online agents where there is space. This list includes all members.
      </p>
      <label>
        Search members
        <input value={query} onChange={(event) => setQuery(event.target.value)} type="search" />
      </label>
      {members.length === 0 ? (
        <p>No {area.binding.type === 'meeting' ? 'members' : 'residents'} yet.</p>
      ) : filtered.length === 0 ? (
        <p>No matching members.</p>
      ) : (
        <ul>
          {filtered.map((member) => (
            <Member key={member.identityId} member={member} areaId={area.id} select={select} />
          ))}
        </ul>
      )}
    </section>
  );
}

export function OfficeDirectory({
  population,
  selection,
  select,
}: {
  population: OfficePopulation;
  selection?: OfficeSelection;
  select: (value: OfficeSelection) => void;
}) {
  const [query, setQuery] = useState('');
  const term = query.trim().toLowerCase();
  const areas = [...population.areas.values()].filter(({ area, room }) =>
    [area.name, area.id, room?.name].some((value) => value?.toLowerCase().includes(term))
  );
  const identities = [...population.identities.values()].filter((profile) =>
    matches({ identityId: profile.identityId, profile }, term)
  );
  return (
    <>
      <label>
        Search directory
        <input value={query} onChange={(event) => setQuery(event.target.value)} type="search" />
      </label>
      <h2>Areas</h2>
      {areas.map(({ area, room }) => (
        <button
          key={area.id}
          aria-pressed={selection?.kind === 'area' && selection.areaId === area.id}
          onClick={() => select({ kind: 'area', areaId: area.id })}
        >
          {area.name} · {area.binding.type}
          {room && <small>{room.name}</small>}
        </button>
      ))}
      {areas.length === 0 && <p>No matching areas.</p>}
      <h2>Agents</h2>
      {population.identities.size === 0 && (
        <p>
          Create an identity with <code>tmt identity create alice</code>. Build and assign an office
          when you want one.
        </p>
      )}
      {identities.map((profile) => {
        const areaId = population.homes.get(profile.identityId)!;
        const home = population.areas.get(areaId)?.area;
        return (
          <button
            key={profile.identityId}
            aria-pressed={
              selection?.kind === 'agent' && selection.identityId === profile.identityId
            }
            onClick={() => select({ kind: 'agent', identityId: profile.identityId, areaId })}
          >
            {profile.identityName} · {presenceLabel[profile.presence]}
            <small>
              {profile.lifetime === 'temporary'
                ? `Contractor · ${home?.name ?? 'Lobby'}`
                : home?.binding.type === 'personal'
                  ? home.name
                  : `Unassigned · ${home?.name ?? 'Lobby'}`}
            </small>
          </button>
        );
      })}
      {population.identities.size > 0 && identities.length === 0 && <p>No matching agents.</p>}
    </>
  );
}
