import { resolveAvatar } from '../avatars/avatar-catalog.js';
import type { AvatarCatalog } from '../avatars/avatar-catalog.js';
import type { OfficePopulation } from './office-population.js';
import type { CatalogPack } from '../props/prop-contract.js';
import type { OfficeSceneModel } from '../rendering/office-scene.js';
import type { SceneAction } from '../rendering/scene-component-geometry.js';
import type { WorldDocument } from '../world-map/world-contract.js';
import { statusCue } from '../identities/identity-status.js';

/** Presence projects into existing areas. Adding an identity never adds floor. */
export function officeSceneModel(
  population: OfficePopulation,
  world: WorldDocument,
  avatars: AvatarCatalog,
  catalog: CatalogPack[],
  components: SceneAction[],
  nowMs: number,
  conversationIdentityId?: string
): OfficeSceneModel {
  const appearances = new Map(
    [...population.identities.values()]
      .filter((profile) => profile.presence === 'active')
      .map((profile) => {
        const art = resolveAvatar(profile.profile.avatarRef, avatars);
        return [
          profile.identityId,
          {
            appearance: profile.profile.appearance,
            name: profile.identityName,
            displayLabel: profile.profile.displayLabel,
            customArt: art.status === 'available' ? art.art : undefined,
          },
        ] as const;
      })
  );
  return {
    world,
    catalog,
    components,
    actors: [...population.areas.values()].flatMap(({ area, members }) =>
      members.flatMap(({ identityId, profile }) => {
        const avatar = appearances.get(identityId);
        return avatar
          ? [
              {
                identityId,
                areaId: area.id,
                contractor: profile?.lifetime === 'temporary',
                avatar,
                activity:
                  identityId === conversationIdentityId
                    ? undefined
                    : statusCue(profile?.selfReportedStatus ?? null, nowMs),
              },
            ]
          : [];
      })
    ),
  };
}
