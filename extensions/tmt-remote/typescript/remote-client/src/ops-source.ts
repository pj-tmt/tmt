/**
 * What the Ops web view (`/ops/`) reads: the owner's squad board through the owner browser read
 * `x/-/ops` (contracts/remote-channel-v1.md, Owner browser reads), read-only. The page depends only
 * on `OpsSource`; this module turns the read's allowlisted projection into the page's rows.
 */
import { ownerRead, type OwnerReadAnswer, type OwnerReadReason } from './owner-reads.js';
/** The shared state marks (c5ccd17a §4): always a mark and a word. The mark is a leaf shape
 * (`tmt-ui-state-mark`), the same box for every state. */
export type AgentState =
  | 'working'
  | 'idle'
  | 'needs-you'
  | 'blocked'
  | 'offline'
  | 'idle-alert'
  | 'unknown';
export type AgentMark = 'dot' | 'ring' | 'dashed' | 'clock' | 'diamond' | 'cross' | 'unknown';
export const AGENT_STATES: Record<AgentState, { mark: AgentMark; word: string; tone: string }> = {
  working: { mark: 'dot', word: 'Working', tone: 'working' },
  idle: { mark: 'dashed', word: 'Idle', tone: 'neutral' },
  'needs-you': { mark: 'diamond', word: 'Needs you', tone: 'waiting' },
  blocked: { mark: 'cross', word: 'Blocked', tone: 'blocked' },
  offline: { mark: 'ring', word: 'Offline', tone: 'neutral' },
  'idle-alert': { mark: 'clock', word: 'Idle alert', tone: 'neutral' },
  unknown: { mark: 'unknown', word: 'Unknown', tone: 'neutral' },
};

export interface OpsAgent {
  name: string;
  squad: string;
  role: 'lead' | 'member';
  state: AgentState;
  /** The board's own state text, shown in place of the mark's word when it says more. */
  stateText?: string;
  /** What the board row says the agent works on, or nothing. */
  task?: string;
  next?: string;
  /** What the agent waits on the person for. */
  pending?: string;
  model?: string;
  /** Requests waiting on the person. */
  waitingOnYou: number;
}
export interface OpsSquad {
  name: string;
  lead?: string;
  /** Minutes before an idle alert, when the squad sets one. */
  idleAlertMinutes?: number;
}
export interface OpsSchedule {
  id: string;
  squad: string;
  /** The schedule as the board shows it, such as `cron 35 * * * *`. */
  when: string;
  what: string;
  owner?: string;
}
export interface OpsBoard {
  agents: readonly OpsAgent[];
  squads: readonly OpsSquad[];
  /** `null` when the owner could not read the schedules: never shown as no timers. */
  schedules: readonly OpsSchedule[] | null;
  /** The owner cut the board to the read's bounds. */
  truncated: boolean;
  /** Why there is no board to show; every list is then empty. */
  unavailable?: string;
}
export interface OpsSource {
  board(signal: AbortSignal): Promise<OpsBoard>;
}

/** The `x/-/ops` payload (contract: Ops payload); every text field is already bounded and inert. */
export interface OpsPayload {
  squads: readonly { name: string; lead?: { id: string; name: string }; idleAlertMs?: number }[];
  agents: readonly {
    name: string;
    squad: string;
    role: 'lead' | 'member';
    presence?: string;
    activity?: { state: string };
    state?: string;
    stateTone?: string;
    task?: string;
    next?: string;
    model?: string;
    pending?: string;
    waitingOnYou: number;
  }[];
  timers:
    | readonly { id: string; squad: string; scheduleText: string; owner?: string; what?: string }[]
    | null;
  truncated: boolean;
}

/** A row's mark: what it owes the person first, then the board's colour token, then activity. */
function agentState(agent: OpsPayload['agents'][number]): AgentState {
  if (agent.presence === 'offline') return 'offline';
  if (agent.pending || agent.waitingOnYou > 0) return 'needs-you';
  if (agent.stateTone === 'blocked') return 'blocked';
  if (agent.stateTone === 'working') return 'working';
  if (agent.stateTone === 'dim') return 'idle';
  const activity = agent.activity?.state;
  return activity === 'working' || activity === 'idle' ? activity : 'unknown';
}

/** One plain sentence per reason, each saying what the person can do. */
const REASONS: Record<OwnerReadReason | 'no-session' | 'unreachable', string> = {
  not_installed:
    'Ops is not installed on your machine. Run `tmt extension install ops`, then reload this page.',
  core_unavailable: 'TMT on your machine did not answer. This page tries again shortly.',
  timeout: 'Your machine took too long to read the board. This page tries again shortly.',
  read_failed: 'Your machine could not read the board. Run `tmt ops ui` in a terminal to see it.',
  too_large: 'The board is too large for the browser. Run `tmt ops ui` in a terminal to see it.',
  not_sampled: 'Your machine could not read the board. Run `tmt ops ui` in a terminal to see it.',
  unknown: 'The board is not available right now. Run `tmt ops ui` in a terminal to see it.',
  'no-session':
    'This browser has no owner session for your machine. Pair it from Remote to see the board.',
  unreachable: 'Your machine did not answer. This page tries again shortly.',
};
function unavailableBoard(reason: OwnerReadReason | 'no-session' | 'unreachable'): OpsBoard {
  return { agents: [], squads: [], schedules: [], truncated: false, unavailable: REASONS[reason] };
}

/** The page's board from one `ok` payload; a payload off the contract is a failed read. */
export function opsBoard(payload: OpsPayload): OpsBoard {
  if (!Array.isArray(payload.squads) || !Array.isArray(payload.agents))
    return unavailableBoard('read_failed');
  return {
    squads: payload.squads.map((squad) => ({
      name: squad.name,
      ...(squad.lead ? { lead: squad.lead.name } : {}),
      ...(squad.idleAlertMs ? { idleAlertMinutes: Math.round(squad.idleAlertMs / 60_000) } : {}),
    })),
    agents: payload.agents.map((agent) => {
      const state = agentState(agent);
      return {
        name: agent.name,
        squad: agent.squad,
        role: agent.role,
        state,
        ...(agent.state && agent.state.toLowerCase() !== AGENT_STATES[state].word.toLowerCase()
          ? { stateText: agent.state }
          : {}),
        ...(agent.task ? { task: agent.task } : {}),
        ...(agent.next ? { next: agent.next } : {}),
        ...(agent.pending ? { pending: agent.pending } : {}),
        ...(agent.model ? { model: agent.model } : {}),
        waitingOnYou: agent.waitingOnYou ?? 0,
      };
    }),
    schedules: Array.isArray(payload.timers)
      ? payload.timers.map((timer) => ({
          id: timer.id,
          squad: timer.squad,
          when: timer.scheduleText,
          what: timer.what ?? '',
          ...(timer.owner ? { owner: timer.owner } : {}),
        }))
      : null,
    truncated: payload.truncated === true,
  };
}

/** The board from an owner read's answer. */
export function boardFromAnswer(answer: OwnerReadAnswer<OpsPayload>): OpsBoard {
  if (answer.state === 'ok') return opsBoard(answer.payload);
  return unavailableBoard(answer.state === 'unavailable' ? answer.reason : answer.state);
}

/** The source the page reads: the owner's board through `x/-/ops`. */
export const opsSource: OpsSource = {
  async board(signal) {
    return boardFromAnswer(await ownerRead<OpsPayload>('ops', signal));
  },
};
