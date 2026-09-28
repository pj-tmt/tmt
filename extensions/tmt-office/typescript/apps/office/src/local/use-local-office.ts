import { useContext, useEffect, useRef, useState } from 'react';
import { LocalRuntimeContext } from './local-runtime.js';
import type { LocalRuntime } from './local-runtime.js';
import type { WorldSnapshot } from '../world-map/world-port.js';
import type { MeetingRoom } from './room-contract.js';
import type { ProfileProjection, ProfileSnapshot } from '../profiles/profile-contract.js';
import type { AvatarCatalog } from '../avatars/avatar-catalog.js';
import type { CatalogPack } from '../props/prop-contract.js';

type OfficeLoad =
  | { status: 'loading'; runtime: LocalRuntime | undefined }
  | { status: 'error'; runtime: LocalRuntime | undefined }
  | {
      status: 'ready';
      runtime: LocalRuntime;
      world: WorldSnapshot;
      rooms: MeetingRoom[];
      profiles: ProfileProjection[];
      avatars: AvatarCatalog;
      props: CatalogPack[];
      refreshing: boolean;
      refreshError?: string;
    };

/** A mounted view's observation, never an alternate durable Office store. */
export function useLocalOffice() {
  const runtime = useContext(LocalRuntimeContext);
  const [generation, setGeneration] = useState(0);
  const [load, setLoad] = useState<OfficeLoad>({ status: 'loading', runtime });
  const refreshingRooms = useRef<
    { runtime: LocalRuntime; updates: Map<string, MeetingRoom> } | undefined
  >(undefined);
  useEffect(() => {
    let active = true;
    if (!runtime) {
      setLoad({ status: 'error', runtime });
      return;
    }
    setLoad((current) =>
      current.status === 'ready' && current.runtime === runtime
        ? { ...current, refreshing: true, refreshError: undefined }
        : { status: 'loading', runtime }
    );
    const controller = new AbortController();
    const roomReads = { runtime, updates: new Map<string, MeetingRoom>() };
    refreshingRooms.current = roomReads;
    void Promise.all([
      runtime.world.show(controller.signal),
      runtime.profiles.list(controller.signal),
      runtime.avatars.list(controller.signal),
      runtime.rooms.list(controller.signal),
    ])
      .then(async ([world, profiles, avatars, rooms]) => {
        if (!active) return;
        const props = await runtime.resolveProps(
          world.layout.objects.map((object) => object.placement),
          controller.signal
        );
        if (active)
          setLoad((current) => {
            // A profile or room can be saved while the refresh is in flight.
            // Keep newer confirmed revisions, without retaining retired identities.
            const previous =
              current.status === 'ready' && current.runtime === runtime ? current : undefined;
            const mergedRooms = new Map(rooms.map((room) => [room.id, room]));
            // Apply observations made after this read began. Effective membership
            // may change at the same revision; new rooms must not disappear either.
            for (const room of roomReads.updates.values()) {
              const fetched = mergedRooms.get(room.id);
              if (!fetched || room.revision >= fetched.revision) mergedRooms.set(room.id, room);
            }
            return {
              status: 'ready',
              runtime,
              world,
              avatars,
              props,
              refreshing: false,
              profiles: profiles.map((profile) => {
                const known = previous?.profiles.find(
                  (item) => item.identityId === profile.identityId
                );
                return known && known.revision > profile.revision
                  ? {
                      ...known,
                      presence: profile.presence,
                      lifetime: profile.lifetime,
                      selfReportedStatus: profile.selfReportedStatus,
                    }
                  : profile;
              }),
              rooms: [...mergedRooms.values()]
                .filter((room) => !room.retired)
                .map((room) => {
                  const known = previous?.rooms.find((item) => item.id === room.id);
                  return known && known.revision > room.revision ? known : room;
                }),
            };
          });
      })
      .catch(() => {
        if (active)
          setLoad((current) =>
            current.status === 'ready' && current.runtime === runtime
              ? {
                  ...current,
                  refreshing: false,
                  refreshError:
                    'Refresh failed. Your open work is kept; the office may be out of date.',
                }
              : { status: 'error', runtime }
          );
      })
      .finally(() => {
        if (refreshingRooms.current === roomReads) refreshingRooms.current = undefined;
      });
    return () => {
      active = false;
      controller.abort();
      if (refreshingRooms.current === roomReads) refreshingRooms.current = undefined;
    };
  }, [runtime, generation]);
  function profileChanged(snapshot: ProfileSnapshot) {
    setLoad((current) =>
      current.status === 'ready' && current.runtime === runtime
        ? {
            ...current,
            profiles: current.profiles.map((item) =>
              item.identityId === snapshot.identityId
                ? {
                    ...snapshot,
                    presence: item.presence,
                    lifetime: item.lifetime,
                    selfReportedStatus: item.selfReportedStatus,
                  }
                : item
            ),
          }
        : current
    );
  }
  function roomChanged(room: MeetingRoom) {
    const pending = refreshingRooms.current;
    if (pending && pending.runtime === runtime) {
      const known = pending.updates.get(room.id);
      if (!known || room.revision >= known.revision) pending.updates.set(room.id, room);
    }
    setLoad((current) => {
      if (current.status !== 'ready' || current.runtime !== runtime) return current;
      const known = current.rooms.find((item) => item.id === room.id);
      // Identity retirement can change effective members without changing the
      // room definition revision. Equal-revision observations are still useful.
      if (known && known.revision > room.revision) return current;
      return {
        ...current,
        rooms: [
          ...current.rooms.filter((item) => item.id !== room.id),
          ...(room.retired ? [] : [room]),
        ],
      };
    });
  }
  function propObserved(pack: CatalogPack) {
    // A confirmed catalog read/install can become a new draft placement before
    // the next overview refresh. Keep it in this mounted observation only.
    setLoad((current) =>
      current.status === 'ready' && current.runtime === runtime
        ? {
            ...current,
            props: [...current.props.filter((item) => item.digest !== pack.digest), pack],
          }
        : current
    );
  }
  return {
    load: load.runtime === runtime ? load : { status: 'loading' as const, runtime },
    refresh: () => setGeneration((value) => value + 1),
    profileChanged,
    roomChanged,
    propObserved,
  };
}
