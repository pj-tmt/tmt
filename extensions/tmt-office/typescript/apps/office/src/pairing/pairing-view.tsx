import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
} from 'react';
import { parsePairingFragment } from './pairing-contract.js';
import type { PairingRequest, PairingPort } from './pairing-contract.js';
import { createPairingState } from './pairing-state.js';
import type { PairingState } from './pairing-state.js';
import { PairingSpaceChoice } from './pairing-space-choice.js';
import './pairing.css';

export const PairingContext = createContext<PairingPort | undefined>(undefined);

export function PairingPanel({
  worldId,
  ownerUid,
  fragment,
}: {
  worldId: string;
  ownerUid: string;
  fragment: string;
}) {
  const port = useContext(PairingContext);
  const request = useMemo(() => {
    try {
      return parsePairingFragment(fragment, worldId);
    } catch {
      return null;
    }
  }, [fragment, worldId]);
  if (!port) return <p role="status">Pairing is not configured for this deployment.</p>;
  if (!request)
    return (
      <p role="alert">Invalid pairing link. Ask for a new link from the requesting installation.</p>
    );
  return (
    <PairingMount
      key={`${ownerUid}:${fragment}`}
      port={port}
      request={request}
      ownerUid={ownerUid}
    />
  );
}

function PairingMount({
  port,
  request,
  ownerUid,
}: {
  port: PairingPort;
  request: PairingRequest;
  ownerUid: string;
}) {
  const [state, setState] = useState<PairingState>();
  useEffect(() => {
    const next = createPairingState(port, request, ownerUid);
    setState(next);
    return () => next.dispose();
  }, [port, request, ownerUid]);
  return state ? <PairingForm state={state} request={request} ownerUid={ownerUid} /> : null;
}

export function PairingForm({
  state,
  request,
  ownerUid,
}: {
  state: PairingState;
  request: PairingRequest;
  ownerUid: string;
}) {
  const { busy, attempted, revocationAttempted, approved, revoked, error } = useSyncExternalStore(
    state.subscribe,
    state.getSnapshot
  );
  const [recognized, setRecognized] = useState(false);
  return (
    <section className="pairing-panel" aria-label="Agent pairing request">
      <p className="eyebrow">INVITE AN AGENT INTO YOUR SPACE</p>
      <h2>Is this your agent?</h2>
      <p>
        Only approve a request you started. Labels are supplied by the requesting installation, not
        verified identities.
      </p>
      <dl className="pairing-details">
        <dt>Agent</dt>
        <dd>{request.identityLabel}</dd>
        <dt>Installation</dt>
        <dd>{request.installationLabel}</dd>
        <dt>Identity ID</dt>
        <dd>
          <code>{request.identityId}</code>
        </dd>
        <dt>Installation ID</dt>
        <dd>
          <code>{request.installationId}</code>
        </dd>
        <dt>Request code</dt>
        <dd>
          <code>{request.pairingId.slice(0, 12)}</code>
        </dd>
        <dt>Access</dt>
        <dd>
          {request.capabilities.length === 2
            ? 'Read and arrange one assigned block'
            : 'Read one assigned block'}
        </dd>
      </dl>
      <p>No notebook, message board, local files or command execution access is granted.</p>
      <p>
        This identity may renew its access in leases of up to 24 hours until you revoke it. Renewal
        keeps the same block and permissions; it does not require daily approval.
      </p>
      <PairingSpaceChoice state={state} request={request} ownerUid={ownerUid} />
      {error && <p role="alert">{error}</p>}
      {approved && (
        <p role="status">
          Approved. Return to the requesting terminal before{' '}
          <time dateTime={new Date(approved.expiresAt).toISOString()}>
            {new Date(approved.expiresAt).toLocaleString()}
          </time>
          . This does not mean the agent is connected.
        </p>
      )}
      {revoked && (
        <p role="status">
          Request {attempted ? 'revoked' : 'cancelled'}. Existing workspace content is retained.
        </p>
      )}
      {!approved && !revoked && !revocationAttempted && (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (recognized) void state.approve();
          }}
        >
          <label className="pairing-consent">
            <input
              type="checkbox"
              checked={recognized}
              disabled={busy}
              onChange={(event) => setRecognized(event.target.checked)}
            />{' '}
            I recognize this agent and installation.
          </label>
          <button disabled={busy || !recognized}>
            {busy ? 'Confirming…' : attempted ? 'Retry same approval' : 'Approve pairing'}
          </button>
        </form>
      )}
      {!revoked && (
        <button type="button" disabled={busy} onClick={() => void state.revoke()}>
          {attempted || approved
            ? 'Revoke request'
            : revocationAttempted
              ? 'Retry cancel request'
              : 'Cancel request'}
        </button>
      )}
    </section>
  );
}
