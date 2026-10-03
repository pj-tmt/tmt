import { test } from '@playwright/test';

// #1110 Ask agent acceptance. Every case below needs code that has not
// landed on colab/1110-ask yet, so it is declared fixme with its dependency;
// a case is enabled by writing its body, never by a passing stand-in. The
// shared world, door, pairing, recipient counter and barriers are in harness/.
//
// Architecture (lead, 2026-10-04): no native bridge ledger. The asker's browser
// calls Remote operations (dispatch.create, operation.show, result, agents.list)
// as its paired device and records the ask, its states and the reply in its
// own Colab stream. v1 is owner-only.
//
// Common precondition for every case: world + door + recipient (harness/), two
// paired browsers (asker, second viewer), a page with known text created through
// `tmt colab page write`, and the recipient named and online.

const ASK_UI = 'Ask panel and Ask agent action (colab-1)';
const REMOTE_SDK = 'browser operations helper in @tmt/remote-client (#1497)';

test.describe('Ask agent real-binary acceptance (#1110)', () => {
  test.fixme('direct send: previewed bytes reach the recipient exactly once and the reply shows in a second viewer', () => {
    // Needs: ASK_UI, REMOTE_SDK.
    // 1. Asker selects page text, presses Ask agent, sees the frozen preview, sends.
    // 2. Recipient durable counter: exactly one `received` row for the request.
    // 3. Exact bytes: received.message === previewed bytes, including the
    //    '[remote: <device-name>]' provenance line, asserted verbatim.
    // 4. Real `tmt reply` row; the reply appears on the asker's page attributed
    //    to the agent, with state accepted, and in the second viewer's page.
    // 5. agents.list shows presence only: the delivery status reads 'unavailable'.
    void [ASK_UI, REMOTE_SDK];
  });

  test.fixme('browser reload recovers the ask via operation.show with the same operation ID and no second wake', () => {
    // Needs: ASK_UI, REMOTE_SDK. Reload while the recipient reply is gated; the
    // page restores the ask from its own stream, shows accepted via
    // operation.show, then the gate releases: one received row, one reply.
  });

  test.fixme('remote restart at the post-core-acceptance barrier recovers via operation.show without a second wake', () => {
    // Needs: ASK_UI, REMOTE_SDK. world.armBarrier(operationId, 'after'); kill
    // tmt-remote while the real core has accepted but not answered; restart it;
    // the page shows uncertain, then accepted with the same operation and
    // request IDs. coreCalls() holds one dispatch.create; recipient has one row.
  });

  test.fixme('remote restart at the pre-dispatch barrier leaves no effect and a same-ID resend delivers once', () => {
    // Needs: ASK_UI, REMOTE_SDK. world.armBarrier(operationId, 'before'); kill
    // and restart tmt-remote; no recipient row before the resend; the resend
    // reuses the operation ID and yields exactly one received row.
  });

  test.fixme('colab restart keeps the ask, its state and the reply from the own stream', () => {
    // Needs: ASK_UI, REMOTE_SDK. Restart tmt-colab between accepted and the reply;
    // after the browser reconnects the reply is attributed and shown once.
  });

  test.fixme('revoking the asker device refuses a later send and creates no recipient work', () => {
    // Needs: ASK_UI, REMOTE_SDK. `tmt-remote devices revoke <client-id>`; the
    // next send shows a refusal; coreCalls() gains no dispatch.create and the
    // recipient counter is unchanged. Positive control: the send worked before.
  });

  test.fixme('a held grant shows held until local approval, then accepted', () => {
    // Needs: ASK_UI, REMOTE_SDK and a hold grant for the device (remote
    // `approve --json` confirms the frozen message). Held creates no recipient row.
  });
});
