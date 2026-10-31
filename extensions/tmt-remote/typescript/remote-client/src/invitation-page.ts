import type { Session } from './device.js';
import {
  invitations,
  type InvitationCandidate,
  type InvitationDecision,
  type InvitationOriginalOutcome,
  type RemoteInvitations,
} from './invitations.js';
import { RefusalError } from './operations.js';

/** One retained presenter. Only explicit decisions write; navigation never repeats one. */
export function invitationPage(
  root: HTMLElement,
  lifetime?: AbortSignal,
  clientForSession: typeof invitations = invitations,
  heldElsewhere: () => boolean = () => false,
  changed: () => void = () => {},
): ((session?: Session) => void) & { heldBy: () => string; update: () => void } {
  const get = <T extends HTMLElement>(id: string): T => root.querySelector<T>(`#${id}`)!;
  const status = get('invitation-status');
  const refresh = get<HTMLButtonElement>('invitation-refresh');
  const form = get('invitation-candidate');
  const compared = get<HTMLInputElement>('invitation-compared');
  const sending = get<HTMLInputElement>('invitation-sending');
  const confirm = get<HTMLButtonElement>('invitation-confirm');
  const decline = get<HTMLButtonElement>('invitation-decline');
  const original = get('invitation-original');
  const originalId = get('invitation-original-id');
  const recover = get<HTMLButtonElement>('invitation-recover');
  let client: RemoteInvitations | undefined;
  let generation = 0;
  let busy = false;
  let current: { offerId: string; candidate: InvitationCandidate } | undefined;
  let unresolved: (InvitationDecision & { clientId: string }) | undefined;
  const here = () =>
    !lifetime?.aborted &&
    root.isConnected &&
    ['/remote/members', '/remote/invites'].includes(location.pathname.replace(/\/$/, ''));
  const controls = (notify = true) => {
    const held = heldElsewhere();
    refresh.disabled = busy || !client || held;
    confirm.disabled = busy || !current || !!unresolved || !compared.checked || held;
    decline.disabled = busy || !current || !!unresolved || held;
    compared.disabled = busy || !!unresolved || held;
    sending.disabled = busy || !!unresolved || held;
    recover.disabled = busy || !client || held;
    if (notify) changed();
  };
  const clear = () => {
    current = undefined;
    form.hidden = true;
    for (const id of ['invitation-name', 'invitation-words', 'invitation-expires'])
      get(id).textContent = '';
    compared.checked = false;
    sending.checked = false;
    controls();
  };
  const uncertain = () => {
    status.textContent =
      'The decision outcome is unconfirmed. Check the original outcome before making another decision. If access ended, check the connection on Overview first.';
    original.hidden = false;
    originalId.textContent = unresolved!.operationId;
    controls();
  };
  const showOutcome = (outcome: InvitationOriginalOutcome) => {
    if (outcome.state === 'unknown') {
      uncertain();
      return;
    }
    if (outcome.state === 'refused') {
      status.textContent =
        outcome.reason === 'REMOTE_INVITATION_DECIDED'
          ? 'This invitation was already decided. Choose Check invitation to read the current outcome.'
          : 'This decision was refused. Choose Check invitation before deciding again.';
    } else {
      const result = outcome.result.invitation;
      if (
        !unresolved ||
        !('clientId' in result) ||
        result.offerId !== unresolved.offerId ||
        result.clientId !== unresolved.clientId ||
        result.state !== (unresolved.matched ? 'confirmed' : 'refused') ||
        (result.state === 'confirmed' && result.sendingEnabled !== unresolved.sending)
      )
        throw new Error('Mismatched invitation outcome.');
      status.textContent =
        result.state === 'confirmed'
          ? `Confirmed; sending ${result.sendingEnabled ? 'on' : 'off'}. The terminal will show this decision.`
          : 'Invitation declined. No member was created by this decision.';
    }
    unresolved = undefined;
    original.hidden = true;
    clear();
  };
  const check = async () => {
    if (!here() || !client || busy || heldElsewhere()) return;
    if (unresolved) {
      uncertain();
      return;
    }
    const turn = ++generation;
    busy = true;
    clear();
    status.textContent = 'Checking the current invitation…';
    controls();
    try {
      const value = await client.pending();
      if (!here() || generation !== turn) return;
      if (value.state === 'none')
        status.textContent =
          'No current invitation. Start tmt remote invite on this machine to invite someone.';
      else if (value.phase === 'awaiting-owner' && value.candidate) {
        current = { offerId: value.offerId, candidate: value.candidate };
        get('invitation-name').textContent = value.candidate.name;
        get('invitation-words').textContent = value.candidate.words.join(' ');
        get('invitation-expires').textContent =
          `Expires ${new Date(value.expiresAtMs).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })}.`;
        status.textContent =
          'Compare these four words with the person joining. The terminal and this browser show the same candidate; the first decision wins.';
        form.hidden = false;
      } else if (value.localConfirmationKnown)
        status.textContent =
          value.phase === 'confirmed'
            ? 'This invitation is confirmed and its receipt was delivered.'
            : 'Confirmed locally; delivery of the original receipt is unconfirmed. Check members before sending another invite.';
      else
        status.textContent =
          (
            {
              open: 'Waiting for someone to join. Check again after they enter the code.',
              refused: 'This invitation was refused.',
              cancelled: 'This invitation was cancelled.',
              expired: 'This invitation expired.',
              unconfirmed:
                'The invitation outcome is unconfirmed. Check member status before sending another invite.',
            } as Record<string, string>
          )[value.phase] ?? 'No candidate is available for confirmation.';
    } catch (error) {
      if (!here() || generation !== turn) return;
      status.textContent =
        error instanceof RefusalError
          ? 'This browser cannot read the invitation. Use the owner browser set up on this machine or confirm in the terminal.'
          : 'The current invitation could not be verified. Check access on Overview and try again.';
    } finally {
      busy = false;
      if (here()) controls();
    }
  };
  const decide = async (matched: boolean) => {
    if (
      !here() ||
      busy ||
      !client ||
      !current ||
      unresolved ||
      heldElsewhere() ||
      (matched && !compared.checked)
    )
      return;
    const turn = ++generation;
    unresolved = {
      operationId: crypto.randomUUID(),
      offerId: current.offerId,
      snapshot: current.candidate.snapshot,
      clientId: current.candidate.clientId,
      matched,
      sending: matched && sending.checked,
    };
    originalId.textContent = unresolved.operationId;
    const input: InvitationDecision = {
      operationId: unresolved.operationId,
      offerId: unresolved.offerId,
      snapshot: unresolved.snapshot,
      matched: unresolved.matched,
      sending: unresolved.sending,
    };
    busy = true;
    controls();
    status.textContent = 'Sending your decision…';
    try {
      const outcome = await client.decide(input);
      if (here() && generation === turn) showOutcome(outcome);
    } catch {
      if (here() && generation === turn) uncertain();
    } finally {
      busy = false;
      if (here()) controls();
    }
  };
  refresh.addEventListener(
    'click',
    () => {
      void check();
    },
    { signal: lifetime },
  );
  compared.addEventListener('change', () => controls(), { signal: lifetime });
  confirm.addEventListener(
    'click',
    () => {
      void decide(true);
    },
    { signal: lifetime },
  );
  decline.addEventListener(
    'click',
    () => {
      void decide(false);
    },
    { signal: lifetime },
  );
  recover.addEventListener(
    'click',
    () => {
      if (!here() || busy || !client || !unresolved || heldElsewhere()) return;
      const turn = ++generation;
      busy = true;
      controls();
      void client
        .operation(unresolved.operationId)
        .then((outcome) => {
          if (here() && generation === turn) showOutcome(outcome);
        })
        .catch(() => {
          if (here() && generation === turn) uncertain();
        })
        .finally(() => {
          busy = false;
          if (here()) controls();
        });
    },
    { signal: lifetime },
  );
  window.addEventListener(
    'popstate',
    () => {
      generation++;
      clear();
      if (here()) {
        if (unresolved) uncertain();
        else void check();
      }
    },
    { signal: lifetime },
  );
  lifetime?.addEventListener(
    'abort',
    () => {
      generation++;
      clear();
      client = undefined;
    },
    { once: true },
  );
  controls();
  return Object.assign(
    (session?: Session) => {
      generation++;
      client = session ? clientForSession(session) : undefined;
      clear();
      if (here()) {
        if (unresolved) uncertain();
        else void check();
      }
    },
    {
      heldBy: () =>
        busy
          ? 'An invitation request is in progress.'
          : unresolved
            ? 'An invitation decision is unconfirmed. Check its original outcome first.'
            : '',
      update: () => controls(false),
    },
  );
}
