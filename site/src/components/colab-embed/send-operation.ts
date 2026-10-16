export type SendOperation = {
  key: string;
  operationId: string;
  message: string;
  pending: boolean;
  done: boolean;
};

type OperationStorage = Pick<Storage, "getItem" | "setItem">;
export const operationStorageKey = "tmt.handbook.embed.v1.send";

export function restoreSendOperation(storage: OperationStorage): SendOperation | null {
  try {
    const value: unknown = JSON.parse(storage.getItem(operationStorageKey) ?? "null");
    if (!value || typeof value !== "object") return null;
    const operation = value as Partial<SendOperation>;
    return typeof operation.key === "string" &&
      typeof operation.operationId === "string" &&
      /^[0-9a-f-]{36}$/.test(operation.operationId) &&
      typeof operation.message === "string" &&
      typeof operation.done === "boolean"
      ? { ...(operation as SendOperation), pending: false }
      : null;
  } catch {
    return null;
  }
}

export function retainSendOperation(
  storage: OperationStorage,
  operation: SendOperation | null,
): void {
  // Persist before dispatch; a reloaded browser retries the same frozen operation.
  storage.setItem(operationStorageKey, JSON.stringify(operation));
}

// These bridge responses are returned before a TMT talk operation is created.
export function definitelyNotSent(status: unknown): boolean {
  return typeof status === "number" && [400, 403, 413, 415, 429, 503].includes(status);
}
