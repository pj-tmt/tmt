/** The fixed Ops rail: observed counts only; the shared leaf owns row presentation. */
import { shellCount, type ShellMenu } from '@tmt/browser-ui/shell';
import type { OpsBoard } from './ops-source.js';

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

export function opsMenu(
  section: OpsSection,
  board: OpsBoard | undefined,
  pathname: string,
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
    ],
    settings: item('settings', 'Ops settings'),
  };
}
