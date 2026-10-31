/** The fixed Ops rail: observed counts only; the shared leaf owns row presentation. */
import { shellCount, type ShellMenu } from '@tmt/browser-ui/shell';
import type { OpsBoard } from './ops-source.js';
import { bySquad } from './ops-parts.js';

export const OPS_SECTIONS = [
  { id: 'agents', label: 'Agents' },
  { id: 'squads', label: 'Squads' },
  { id: 'schedules', label: 'Timers' },
  { id: 'settings', label: 'Ops settings' },
] as const;
export type OpsSection = (typeof OPS_SECTIONS)[number]['id'];

export function opsSection(hash: string): OpsSection {
  return OPS_SECTIONS.find((section) => `#${section.id}` === hash)?.id ?? 'agents';
}

export type OpsPlace = OpsSection | { squad: string; memberId?: string };

export function opsPlace(hash: string): OpsPlace {
  try {
    if (hash.startsWith('#channel/')) {
      const squad = decodeURIComponent(hash.slice(9));
      if (squad) return { squad };
    }
    const dm = /^#dm\/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\/(.+)$/.exec(
      hash,
    );
    if (dm) return { memberId: dm[1]!, squad: decodeURIComponent(dm[2]!) };
  } catch {
    /* A malformed link never selects a member by name. */
  }
  return opsSection(hash);
}

export function opsHref(pathname: string, place: OpsPlace): string {
  if (typeof place === 'string') return `${pathname}#${place}`;
  const squad = encodeURIComponent(place.squad);
  return place.memberId
    ? `${pathname}#dm/${place.memberId}/${squad}`
    : `${pathname}#channel/${squad}`;
}

export function opsMenu(
  section: OpsPlace,
  board: OpsBoard | undefined,
  pathname: string,
  disclosure?: { collapsed: ReadonlySet<string>; toggle(squad: string): void },
): ShellMenu {
  // A bounded subset cannot establish a total, and an unavailable read is not an empty board.
  const observed = board && !board.unavailable && !board.truncated ? board : undefined;
  const totals = {
    agents: observed?.agents.length,
    squads: observed?.squads.length,
    schedules: observed?.schedules?.length,
  };
  // The Agents view and its state strip use this same projected state, not a presence guess.
  const attention = observed?.agents.filter((agent) => agent.state === 'needs-you').length;
  const item = (id: OpsSection, label: string) => ({
    id,
    label,
    href: `${pathname}#${id}`,
    current: section === id,
  });
  return {
    title: 'Ops',
    groups: [
      {
        items: OPS_SECTIONS.filter((entry) => entry.id !== 'settings').map((entry) => ({
          ...item(entry.id, entry.label),
          total: totals[entry.id],
          count: entry.id === 'agents' && attention ? shellCount(attention) : undefined,
        })),
      },
      // Expansion belongs to this route; the shared leaf renders the same tree on phones.
      ...(!board?.unavailable && board
        ? bySquad(board, true).map((group) => ({
            items: [
              {
                id: `channel:${group.name}`,
                label: `#${group.name}`,
                href: opsHref(pathname, { squad: group.name }),
                current:
                  typeof section !== 'string' && section.squad === group.name && !section.memberId,
                expanded: !disclosure?.collapsed.has(group.name),
                toggle: disclosure ? () => disclosure.toggle(group.name) : undefined,
                children: group.agents.map((agent) => ({
                  id: `dm:${group.name}:${agent.id}`,
                  label: agent.name,
                  href: opsHref(pathname, { squad: group.name, memberId: agent.id }),
                  current:
                    typeof section !== 'string' &&
                    section.squad === group.name &&
                    section.memberId === agent.id,
                })),
              },
            ],
          }))
        : []),
    ],
    settings: item('settings', 'Ops settings'),
  };
}
