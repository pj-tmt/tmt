import type { ReactElement } from 'react';

export function SetupPage(): ReactElement {
  return (
    <section>
      <p className="eyebrow">Foundation preview</p>
      <h1>
        Start small.
        <br />
        Stay in control.
      </h1>
      <p className="intro">The native TMT CLI works independently of this app.</p>
      <ol className="setup-steps">
        <li>
          <h2>A private world</h2>
          <p>
            Invitations will control who can visit. Local emulator sign-in alone grants no world
            access.
          </p>
        </li>
        <li>
          <h2>Your local team</h2>
          <p>
            An optional connector will bring your agents online with explicit pairing and
            permissions. Not installed by this app.
          </p>
        </li>
        <li>
          <h2>Work by invitation</h2>
          <p>Visiting or chatting will not grant permission to run commands on your computer.</p>
        </li>
      </ol>
    </section>
  );
}
