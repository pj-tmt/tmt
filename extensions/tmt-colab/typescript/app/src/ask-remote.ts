/** Contract-shaped port, not a second Remote SDK. Production adoption waits
 * for #1055; no implementation here reads Remote keys or calls its routes. */
export type Delivery = 'channel' | 'paste' | 'not_ready' | 'not_running';
export interface RemoteAgent {
  id: string;
  name: string;
  delivery?: Delivery;
}
export interface SendInput {
  operationId: string;
  agentId: string;
  message: string;
}
export type SendState =
  | { state: 'held'; operationId: string }
  | { state: 'accepted'; operationId: string; requestId: string }
  | { state: 'uncertain'; operationId: string; requestId?: string }
  | { state: 'refused' | 'cancelled'; operationId: string; reason?: string };
export type ResultState =
  | { state: 'pending'; requestId: string }
  | { state: 'replied'; requestId: string; message: string }
  | { state: 'unavailable'; requestId: string; reason?: string };
export interface RemoteClient {
  listAgents(): Promise<RemoteAgent[]>;
  send(input: SendInput): Promise<SendState>;
  operation(operationId: string): Promise<SendState>;
  check(agentId: string): Promise<unknown>;
  result(requestId: string): Promise<ResultState>;
}
