# Extension serve lifecycle

How an extension's `serve` command starts in the background, hands over readiness and stops. It
applies to every product that runs a long-lived local serve (today `tmt remote serve` and
`tmt colab serve`) and is implemented once by the
[`tmt-extension-serve`](../rust/crates/tmt-extension-serve/src/lib.rs) leaf crate. A product
contract states only what it adds: its readiness record, its codes and wording, its limits and its
failure-record phases.

## Modes

A human invocation starts a detached serve and returns after verified readiness. `--background`
does the same with `--json` and prints one typed ready or error result. `--foreground` keeps direct
terminal ownership until Ctrl-C or SIGTERM. The mode flags are mutually exclusive. A bare `--json`
stays foreground, so a supervisor that starts the process keeps owning it.

## Handoff

- **Exact worker.** The launcher starts only its own executable again, as the worker, with a
  private Unix socket pair as the worker's stdin. The worker starts its own session (closing the
  terminal does not stop it), takes the pair, closes inherited stdin and runs the same foreground
  composition as a foreground serve. The launcher performs no discovery or state initialization of
  its own.
- **Readiness.** The worker sends one `Ready` record after it owns every resource a started serve
  owns (the product decides which) and before it serves. Serving starts only after the launcher's
  one-byte `Accept`. The worker answers `Accepted` once; a lost acknowledgment never revokes the
  handoff.
- **Bounded startup.** One absolute startup deadline covers every frame, including partial headers
  and payloads. Records are typed and byte-bounded. The deadline is not a total command-duration or
  filesystem-I/O promise.

## Cancellation and the cutoff

- **Before `Accept`.** Cancellation, EOF, a deadline or an invalid record make the worker clean up
  and the launcher wait, within the cleanup wait, for its own worker's confirmed exit. A worker that
  cannot confirm cleanup is reported as startup unconfirmed: the person inspects status and uses
  stop, and nothing starts again automatically.
- **The cutoff.** The launcher's successful `Accept` write is its no-kill cutoff. A cancellation
  observed earlier wins; a signal racing that write may lose to the acceptance. The worker consumes
  a buffered `Accept` before a later EOF, preserves actual shutdown signals, attempts `Accepted`
  once and closes the startup endpoint.
- **After `Accept`.** The launcher never cleans up, kills or retries. Lost acknowledgment or lost
  output is reported as startup unconfirmed too, never as proof that the serve is gone. Partial
  readiness output is never followed by a second error record.
- **No kill authority from discovery.** Only the exact child the launcher spawned and has not reaped
  is ever signalled; no PID or status lookup supplies authority, and no signal follows a reap or an
  acceptance. Forced termination, a crash or an inherited lease cannot prove that every descendant
  ended; that uncertainty is reported, never assumed away.

## Second start, failure record and stop

- **Second start.** A start that finds the serve lock held reports the running serve and starts
  nothing. A product maps that to its own already-serving error, and may present it as success for
  a person at a terminal while `--json` keeps the error.
- **Failure record.** After the worker holds the serve lock it may clear and then write one bounded,
  sanitized failure record for a failure the launcher can no longer receive. It never appends or
  retries, and a missing record does not prove a healthy exit.
- **Stop.** Stopping goes through the product's existing stop owner, never a signal to a PID. What a
  serve started itself (such as a door) stops with it; what it only attached to keeps running.

## What a product supplies

The crate has no product names, paths, ports or policy. A product passes its program and
arguments, the readiness validator, the failure-record file, its `unconfirmed` and `cancelled`
errors and its timing bounds. Everything a person reads, and every code, stays in the product's
own contract.
