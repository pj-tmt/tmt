export function retryReleaseRead<T>(
  read: () => T,
  options: {
    label: string;
    retryIf?: (failure: unknown) => boolean;
    sleep?: (milliseconds: number) => void;
  }
): T;
