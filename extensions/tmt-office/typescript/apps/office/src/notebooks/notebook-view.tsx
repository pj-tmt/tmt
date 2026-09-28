import { useEffect, useState } from 'react';
import { LocalHttpError } from '../local/local-runtime.js';
import type { Notebook, NotebookPort } from './notebook-contract.js';
import './notebook.css';

type ReadState =
  | { kind: 'loading' }
  | { kind: 'ready'; notebook: Notebook }
  | { kind: 'error'; message: string };

function readError(cause: unknown): string {
  if (cause instanceof LocalHttpError) {
    switch (cause.code) {
      case 'NOTEBOOK_NOT_FOUND':
        return 'No notebook exists yet. The agent can create one with tmt notes path.';
      case 'NOTEBOOK_IDENTITY_NOT_FOUND':
        return 'This identity is no longer active. Existing notes are retained on disk.';
      case 'NOTEBOOK_SAVED_IDENTITY_REQUIRED':
        return 'Only saved agents have persistent notebooks. Contractors do not.';
      case 'NOTEBOOK_TOO_LARGE':
        return 'This notebook exceeds the 1 MiB viewer limit. The source file is unchanged.';
      case 'NOTEBOOK_INVALID_TEXT':
        return 'This notebook is not valid UTF-8. The source file is unchanged.';
    }
  }
  return 'The notebook could not be read. Refresh to try again; the source file is unchanged.';
}

/** Mounted per identity and per open panel; closing releases the read and its content. */
export function NotebookView({ identityId, port }: { identityId: string; port: NotebookPort }) {
  const [attempt, refresh] = useState(0);
  const [state, setState] = useState<ReadState>({ kind: 'loading' });
  useEffect(() => {
    const lifetime = new AbortController();
    setState({ kind: 'loading' });
    void port.read(identityId, lifetime.signal).then(
      (notebook) => {
        if (!lifetime.signal.aborted) setState({ kind: 'ready', notebook });
      },
      (cause: unknown) => {
        if (!lifetime.signal.aborted) setState({ kind: 'error', message: readError(cause) });
      }
    );
    return () => lifetime.abort();
  }, [identityId, port, attempt]);

  return (
    <section
      className="notebook-view"
      aria-label="Agent notebook"
      aria-busy={state.kind === 'loading'}
    >
      <h2>{state.kind === 'ready' ? `${state.notebook.name}'s notebook` : 'Agent notebook'}</h2>
      <p>Read-only · the agent owns the Markdown file. Refresh to see its latest notes.</p>
      <p className="notebook-identity">{identityId}</p>
      <button disabled={state.kind === 'loading'} onClick={() => refresh((value) => value + 1)}>
        Refresh notebook
      </button>
      {state.kind === 'loading' && <p role="status">Reading notebook…</p>}
      {state.kind === 'error' && <p role="alert">{state.message}</p>}
      {state.kind === 'ready' &&
        (state.notebook.content.length === 0 ? (
          <p>The notebook is empty.</p>
        ) : (
          <pre tabIndex={0} aria-label="Notebook content">
            {state.notebook.content}
          </pre>
        ))}
    </section>
  );
}
