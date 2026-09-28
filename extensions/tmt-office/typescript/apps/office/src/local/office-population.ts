import type { ProfileProjection } from '../profiles/profile-contract.js';
import type { MapArea } from '../world-map/map-contract.js';
import { mapGeometry } from '../world-map/map-source.js';
import type { MapSource } from '../world-map/map-source.js';
import type { MeetingRoom } from './room-contract.js';

export interface AreaMember {
  identityId: string;
  profile?: ProfileProjection;
}
export interface AreaPopulation {
  area: MapArea;
  room?: MeetingRoom;
  members: AreaMember[];
}
export type OfficePopulation = ReturnType<typeof officePopulation>;

/** Read-only projections. Room membership never moves or duplicates an identity's home. */
export function officePopulation(
  source: MapSource,
  profiles: ProfileProjection[],
  rooms: MeetingRoom[]
) {
  const map = mapGeometry(source);
  const identities = new Map(profiles.map((profile) => [profile.identityId, profile]));
  const meetings = new Map(rooms.map((room) => [room.id, room]));
  const homes = new Map(profiles.map((profile) => [profile.identityId, map.primaryLobbyId]));
  for (const area of map.areas)
    if (
      area.binding.type === 'personal' &&
      area.binding.identityId &&
      identities.get(area.binding.identityId)?.lifetime === 'saved'
    )
      homes.set(area.binding.identityId, area.id);

  const areas = new Map<string, AreaPopulation>(
    map.areas.map((area) => {
      const room = area.binding.type === 'meeting' ? meetings.get(area.binding.roomId) : undefined;
      const ids =
        area.binding.type === 'meeting'
          ? (room?.memberIds ?? [])
          : profiles
              .filter((profile) => homes.get(profile.identityId) === area.id)
              .map((p) => p.identityId);
      return [
        area.id,
        {
          area,
          room,
          members: [...new Set(ids)].sort().map((identityId) => ({
            identityId,
            profile: identities.get(identityId),
          })),
        },
      ];
    })
  );
  return { identities, homes, areas };
}
