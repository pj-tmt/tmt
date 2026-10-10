/** Retries only an acquisition read; validation and writes stay with the caller. */
export function retryReleaseRead(
  read,
  {
    label,
    retryIf = () => true,
    sleep = (milliseconds) =>
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds),
  }
) {
  for (let attempt = 1; ; attempt += 1) {
    try {
      return read();
    } catch (failure) {
      if (attempt === 3 || !retryIf(failure)) throw failure;
      const delay = 1000 * 2 ** (attempt - 1);
      process.stderr.write(
        `${label} attempt ${attempt}/3 failed: ${failure.message}\nRetrying in ${delay} ms.\n`
      );
      sleep(delay);
    }
  }
}
