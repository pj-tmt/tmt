import Database from 'better-sqlite3';

/** Interpret one real hook completion; callers retain their event/log ownership. */
export function hookResult(error, stdout, stderr) {
  if (error) throw error;
  const warning = 'tmt: lifecycle context unavailable; continuing without context.';
  const diagnostic = stderr === warning || stderr === `${warning}\n`;
  if (stderr && !diagnostic) throw new Error(stderr);
  if (stdout) JSON.parse(stdout);
  return diagnostic ? { stdout, diagnostic: stderr } : { stdout };
}

/** Read-only fixture admission, shared by each launch phase. */
export async function waitForLifecycle(databasePath, parentPid, sessionId, phase, deadline) {
  for (;;) {
    const database = new Database(databasePath, { readonly: true });
    let row;
    try {
      row = database
        .prepare(
          `SELECT b.runtime_state,b.runtime_pid,b.runtime_start_identity,
                  b.observed_provider_session_id,p.remembered_harness,
                  p.provider_session_id,p.driver_state
           FROM bindings b LEFT JOIN identity_session_preferences p ON p.identity_id=b.identity_id
           WHERE b.runtime_pid = ?`
        )
        .get(parentPid);
    } finally {
      database.close();
    }
    const running = row?.runtime_state === 'running';
    const started =
      running &&
      row.observed_provider_session_id === sessionId &&
      row.remembered_harness === 'claude' &&
      row.provider_session_id === sessionId;
    let activity;
    try {
      activity = row?.driver_state ? JSON.parse(row.driver_state).activity : undefined;
    } catch {
      // An unreadable snapshot never supplies fixture readiness.
    }
    const working =
      started &&
      activity?.session === sessionId &&
      activity.pid === parentPid &&
      activity.start === row.runtime_start_identity &&
      activity.state === 'working';
    if (
      (phase === 'running' && running) ||
      (phase === 'started' && started) ||
      (phase === 'working' && working)
    )
      return;
    if (Date.now() >= deadline)
      throw new Error(`native fixture ${phase} was not committed: ${JSON.stringify(row)}`);
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
}
