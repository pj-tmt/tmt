import { useState } from 'react';
import type { ProfileProjection, ProfileSnapshot } from '../profiles/profile-contract.js';
import type { AvatarCatalog } from '../avatars/avatar-catalog.js';
import { resolveAvatar } from '../avatars/avatar-catalog.js';
import { Avatar } from '../profiles/avatar.js';
import { ProfilePanel } from '../profiles/profile-view.js';
import type { ProfilePort } from '../profiles/profile-contract.js';
import type { ReactNode } from 'react';
import { isStatusFresh } from '../identities/identity-status.js';
import { presenceLabel } from '../identities/presence.js';

function StatusTime({ value }: { value: number }) {
  const date = new Date(value);
  // The wire permits safe integers beyond the browser Date range.
  return Number.isFinite(date.getTime()) ? (
    <time dateTime={date.toISOString()}>{date.toLocaleString()}</time>
  ) : (
    <span>{value} ms since epoch</span>
  );
}

export function AgentPortrait({
  profile,
  catalog,
}: {
  profile: ProfileProjection;
  catalog: AvatarCatalog;
}) {
  const custom = resolveAvatar(profile.profile.avatarRef, catalog);
  return (
    <svg
      className="agent-hud-portrait"
      viewBox="-4 -4 8 8"
      aria-label={`${profile.identityName} portrait`}
    >
      <Avatar
        name={profile.identityName}
        appearance={profile.profile.appearance}
        showName={false}
        customArt={custom.status === 'available' ? custom.art : undefined}
      />
    </svg>
  );
}

export function AgentInfo({
  profile,
  port,
  catalog,
  changed,
  nowMs,
  children,
}: {
  profile: ProfileProjection;
  port: ProfilePort;
  catalog: AvatarCatalog;
  changed(value: ProfileSnapshot): void;
  nowMs: number;
  children?: ReactNode;
}) {
  const [appearance, showAppearance] = useState(false);
  return (
    <>
      <p>
        {profile.lifetime === 'temporary' ? 'Contractor' : 'Saved identity'} ·{' '}
        {presenceLabel[profile.presence]}
      </p>
      <p>{profile.profile.description}</p>
      <section className="agent-self-status" aria-label="Self-reported status">
        <strong>Self-reported status</strong>
        {profile.selfReportedStatus ? (
          <>
            <span>{isStatusFresh(profile.selfReportedStatus, nowMs) ? 'Current' : 'Stale'}</span>
            <p>{profile.selfReportedStatus.activity}</p>
            {profile.selfReportedStatus.mood && <p>Mood: {profile.selfReportedStatus.mood}</p>}
            <small>
              Updated <StatusTime value={profile.selfReportedStatus.updatedAtMs} />
            </small>
            <small>
              Expires <StatusTime value={profile.selfReportedStatus.expiresAtMs} />
            </small>
          </>
        ) : (
          <p>No status shared.</p>
        )}
      </section>
      <button aria-expanded={appearance} onClick={() => showAppearance(!appearance)}>
        Appearance
      </button>
      {appearance && (
        <ProfilePanel
          key={profile.identityId}
          initial={profile}
          port={port}
          changed={changed}
          avatarCatalog={catalog}
        />
      )}
      {children}
    </>
  );
}
