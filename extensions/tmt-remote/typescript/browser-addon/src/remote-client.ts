// Local UI port agreed with tmt-remote-oai; this is not the wire protocol or SDK.
export interface Agent {
  id: string;
  name: string;
}
export interface SendInput {
  operationId: string;
  agentId: string;
  message: string;
}
export type SendState =
  | { operationId: string; state: 'held' }
  | { operationId: string; state: 'accepted'; requestId: string }
  | { operationId: string; state: 'uncertain'; requestId?: string }
  | { operationId: string; state: 'refused' | 'cancelled'; reason?: string };
export type ResultState =
  | { requestId: string; state: 'pending' }
  | { requestId: string; state: 'replied'; message: string }
  | { requestId: string; state: 'unavailable'; reason?: string };
export interface ClientError {
  code: 'unpaired' | 'closed' | 'scope_denied' | 'rate_limited' | 'input_invalid' | 'unavailable';
  message: string;
  retryAfterMs?: number;
}
export interface RemoteClient {
  listAgents(): Promise<Agent[]>;
  send(input: SendInput): Promise<SendState>;
  operation(operationId: string): Promise<SendState>;
  result(requestId: string): Promise<ResultState>;
}
