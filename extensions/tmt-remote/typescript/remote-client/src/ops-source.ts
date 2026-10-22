/**
 * What the Ops web view (`/ops/`) reads: the squad board's agents, squads, schedules and settings,
 * read-only. The page depends only on `OpsSource`; the source below is a MOCK until Core exposes a
 * read API for the board (`tmt ops ui` reads the same facts in the terminal).
 */
import { notInBrowser } from './not-available.js';
/** The shared state marks (c5ccd17a §4): always a mark and a word. */
export type AgentState =
  | 'working'
  | 'idle'
  | 'needs-you'
  | 'blocked'
  | 'offline'
  | 'idle-alert'
  | 'unknown';
export const AGENT_STATES: Record<AgentState, { mark: string; word: string; tone: string }> = {
  working: { mark: '●', word: 'Working', tone: 'working' },
  idle: { mark: '◌', word: 'Idle', tone: 'neutral' },
  'needs-you': { mark: '◆', word: 'Needs you', tone: 'waiting' },
  blocked: { mark: '✗', word: 'Blocked', tone: 'blocked' },
  offline: { mark: '○', word: 'Offline', tone: 'neutral' },
  'idle-alert': { mark: '◷', word: 'Idle alert', tone: 'neutral' },
  unknown: { mark: '?', word: 'Unknown', tone: 'neutral' },
};

export interface OpsAgent {
  name: string;
  squad: string;
  role: 'lead' | 'member';
  state: AgentState;
  /** What the board row says the agent works on, or nothing. */
  task?: string;
  model?: string;
}
export interface OpsSquad {
  name: string;
  lead: string;
  /** Minutes before an idle alert, when the squad sets one. */
  idleAlertMinutes?: number;
}
export interface OpsSchedule {
  id: string;
  squad: string;
  /** The schedule as the board shows it, such as `hourly :07`. */
  when: string;
  what: string;
}
export interface OpsBoard {
  agents: readonly OpsAgent[];
  squads: readonly OpsSquad[];
  schedules: readonly OpsSchedule[];
  settings: readonly { name: string; value: string; description: string }[];
  /** Whether this is sample data, not the owner's board. */
  sample: boolean;
  /** Why there is no board to show; every list is then empty. */
  unavailable?: string;
}
export interface OpsSource {
  board(): Promise<OpsBoard>;
}

// MOCK: a small fixed board so the view can be built and reviewed before the Core read API exists.
// Replace with the API-backed source; nothing else in the page changes.
export const mockOpsSource: OpsSource = {
  async board() {
    return {
      sample: true,
      squads: [
        { name: 'tmt-ux', lead: 'tmt-ux-lead', idleAlertMinutes: 30 },
        { name: 'tmt-feat', lead: 'tmt-feat-lead', idleAlertMinutes: 30 },
        { name: 'tmt-remote', lead: 'tmt-remote-lead' },
      ],
      agents: [
        {
          name: 'tmt-ux-lead',
          squad: 'tmt-ux',
          role: 'lead',
          state: 'working',
          task: 'Night shift: live review',
          model: 'opus',
        },
        {
          name: 'ux-1',
          squad: 'tmt-ux',
          role: 'member',
          state: 'working',
          task: '#3931 input grows with content',
          model: 'sonnet',
        },
        {
          name: 'ux-2',
          squad: 'tmt-ux',
          role: 'member',
          state: 'needs-you',
          task: 'Asks: Shift+Enter behavior',
          model: 'sol',
        },
        { name: 'tmt-feat-lead', squad: 'tmt-feat', role: 'lead', state: 'idle', model: 'opus' },
        {
          name: 'feat-1',
          squad: 'tmt-feat',
          role: 'member',
          state: 'idle-alert',
          task: 'Idle 32m',
          model: 'sonnet',
        },
        {
          name: 'tmt-remote-lead',
          squad: 'tmt-remote',
          role: 'lead',
          state: 'offline',
          model: 'opus',
        },
        {
          name: 'remote-1',
          squad: 'tmt-remote',
          role: 'member',
          state: 'blocked',
          task: 'Waiting on CI',
          model: 'sol',
        },
      ],
      schedules: [
        { id: 'c1', squad: 'tmt-infra', when: 'hourly :07', what: 'Release health tick' },
        { id: 'c2', squad: 'tmt-infra', when: 'Mon 10:00', what: 'Guidance hygiene pass' },
      ],
      settings: [
        {
          name: 'Board refresh',
          value: '5s',
          description: 'How often the board re-reads agent state',
        },
        { name: 'Idle alert default', value: '30m', description: 'Used when a squad sets none' },
      ],
    };
  },
};

/** The source the page reads by default. */
// Needs a Core read of the squad board served to the owner's browser.
export const opsSource: OpsSource = {
  async board() {
    return {
      agents: [],
      squads: [],
      schedules: [],
      settings: [],
      sample: false,
      unavailable: notInBrowser('The Ops board', 'tmt ops ui'),
    };
  },
};
