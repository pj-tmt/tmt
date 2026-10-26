import {
  ClientError,
  RefusalError,
  verifiedSessionRequest,
  type RemoteRefusalCode,
} from './operations.js';

export interface ManagementRefused {
  operationId: string;
  state: 'refused';
  reason: RemoteRefusalCode;
}
/** Both management facades share the existing verified signing lane and never retry effects. */
export async function managementMutation<T>(
  call: ReturnType<typeof verifiedSessionRequest>,
  operation: string,
  value: { operationId: string },
  decode: (value: unknown) => T,
): Promise<T | ManagementRefused> {
  const { operationId } = value;
  try {
    return await call(operation, operationId, value, decode, true);
  } catch (error) {
    if (error instanceof ClientError) throw new ClientError(error.code, error.message, operationId);
    if (error instanceof RefusalError) return { state: 'refused', operationId, reason: error.code };
    throw error;
  }
}
