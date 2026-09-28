import { useEffect, useState } from 'react';
import type { Profile, ProfilePort, ProfileSnapshot } from './profile-contract.js';
import {
  PROFILE_CATALOG,
  ProfileAvatarUnavailable,
  ProfileConflict,
  validProfile,
} from './profile-contract.js';
import { Avatar } from './avatar.js';
import type { AvatarCatalog } from '../avatars/avatar-catalog.js';
import { avatarOptions, resolveAvatar, validAvatarReference } from '../avatars/avatar-catalog.js';
import './profile.css';

const draftKey = (id: string) => `tmt-office-profile-draft:${id}`;
const DRAFT_BYTES = 16 * 1024;
interface ProfileDraft {
  baseRevision: number;
  profile: Profile;
}
const EMPTY_AVATAR_CATALOG: AvatarCatalog = { catalogRevision: 0, packs: [] };
const fieldLabel = {
  hairStyle: 'Hair style',
  hairColor: 'Hair color',
  skinTone: 'Skin tone',
  shirtColor: 'Shirt color',
} as const;
function editableProfile(value: unknown): value is Profile {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const profile = value as Record<string, unknown>;
  const keys = Object.keys(profile).sort().join(',');
  if (
    keys !== 'appearance,description,displayLabel' &&
    keys !== 'appearance,avatarRef,description,displayLabel'
  )
    return false;
  if (
    typeof profile.displayLabel !== 'string' ||
    typeof profile.description !== 'string' ||
    !profile.appearance ||
    typeof profile.appearance !== 'object' ||
    Array.isArray(profile.appearance)
  )
    return false;
  const appearance = profile.appearance as Record<string, unknown>;
  return (
    Object.keys(appearance).sort().join(',') ===
      'hairColor,hairStyle,shirtColor,shirtMark,skinTone' &&
    typeof appearance.shirtMark === 'string' &&
    (profile.avatarRef === undefined || validAvatarReference(profile.avatarRef)) &&
    PROFILE_CATALOG.hairStyles.includes(appearance.hairStyle as never) &&
    PROFILE_CATALOG.hairColors.includes(appearance.hairColor as never) &&
    PROFILE_CATALOG.skinTones.includes(appearance.skinTone as never) &&
    PROFILE_CATALOG.shirtColors.includes(appearance.shirtColor as never)
  );
}
function restoredDraft(snapshot: ProfileSnapshot): ProfileDraft {
  try {
    const encoded = localStorage.getItem(draftKey(snapshot.identityId));
    if (!encoded || new TextEncoder().encode(encoded).length > DRAFT_BYTES)
      throw new Error('No bounded draft.');
    const value: unknown = JSON.parse(encoded);
    if (!value || typeof value !== 'object' || Array.isArray(value))
      throw new Error('Invalid draft.');
    const draft = value as Record<string, unknown>;
    if (
      Object.keys(draft).sort().join(',') === 'baseRevision,profile' &&
      Number.isSafeInteger(draft.baseRevision) &&
      (draft.baseRevision as number) >= 0 &&
      editableProfile(draft.profile)
    )
      return draft as unknown as ProfileDraft;
  } catch {
    /* fall through to the current saved state */
  }
  return { baseRevision: snapshot.revision, profile: snapshot.profile };
}
export function ProfilePanel({
  initial,
  port,
  changed,
  avatarCatalog = EMPTY_AVATAR_CATALOG,
}: {
  initial: ProfileSnapshot;
  port: ProfilePort;
  changed(snapshot: ProfileSnapshot): void;
  avatarCatalog?: AvatarCatalog;
}) {
  const [remote, setRemote] = useState(initial);
  const [draft, setDraft] = useState<ProfileDraft>(() => restoredDraft(initial));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [storageError, setStorageError] = useState<string>();
  const avatar = resolveAvatar(draft.profile.avatarRef, avatarCatalog);
  const options = avatarOptions(avatarCatalog);
  useEffect(() => {
    try {
      if (
        draft.baseRevision === remote.revision &&
        JSON.stringify(draft.profile) === JSON.stringify(remote.profile)
      )
        localStorage.removeItem(draftKey(initial.identityId));
      else {
        const encoded = JSON.stringify(draft);
        if (new TextEncoder().encode(encoded).length > DRAFT_BYTES)
          throw new Error('Draft is too large.');
        localStorage.setItem(draftKey(initial.identityId), encoded);
      }
      setStorageError(undefined);
    } catch {
      setStorageError('This draft could not be saved in this browser. Keep this page open.');
    }
  }, [draft, initial.identityId, remote.profile, remote.revision]);
  const setAppearance = (key: keyof Profile['appearance'], value: string) =>
    setDraft((current) => ({
      ...current,
      profile: {
        ...current.profile,
        appearance: { ...current.profile.appearance, [key]: value },
      },
    }));
  const setAvatar = (value: string) =>
    setDraft((current) => {
      if (value) return { ...current, profile: { ...current.profile, avatarRef: value } };
      const { avatarRef: _avatarRef, ...profile } = current.profile;
      return { ...current, profile };
    });
  async function save() {
    if (!validProfile(draft.profile) || busy) {
      setError('Keep profile text within the limits and choose catalog values.');
      return;
    }
    const intent = draft;
    setBusy(true);
    setError(undefined);
    try {
      const { changed: _changed, ...saved } = await port.apply(
        initial.identityId,
        intent.baseRevision,
        intent.profile
      );
      setRemote(saved);
      setDraft({ baseRevision: saved.revision, profile: saved.profile });
      try {
        localStorage.removeItem(draftKey(initial.identityId));
      } catch {
        setStorageError('This draft could not be cleared from this browser.');
      }
      changed(saved);
    } catch (cause) {
      if (cause instanceof ProfileConflict || cause instanceof ProfileAvatarUnavailable)
        setError(cause.message);
      else
        setError('Save could not be confirmed. Your draft is preserved; reread before retrying.');
      try {
        const latest = await port.show(initial.identityId);
        setRemote(latest);
        changed(latest);
      } catch {
        /* draft remains authoritative locally when reread also fails */
      }
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="profile-editor" aria-label="Agent appearance editor">
      <div className="profile-preview">
        <svg viewBox="-8 -7 16 15" role="img">
          <Avatar
            appearance={draft.profile.appearance}
            name={remote.identityName}
            displayLabel={draft.profile.displayLabel}
            customArt={avatar.status === 'available' ? avatar.art : undefined}
          />
        </svg>
        <p>
          {remote.exists
            ? `Saved · revision ${remote.revision}`
            : 'Deterministic default · revision 0'}
        </p>
        <p className={`avatar-status avatar-status-${avatar.status}`}>
          {avatar.status === 'default'
            ? 'Avatar · default robot'
            : avatar.status === 'available'
              ? `Avatar · ${avatar.label}`
              : 'Avatar · unavailable, showing saved default appearance'}
        </p>
      </div>
      <fieldset className="profile-fields" disabled={busy}>
        <label>
          Display label
          <input
            value={draft.profile.displayLabel}
            onChange={(event) =>
              setDraft({
                ...draft,
                profile: { ...draft.profile, displayLabel: event.target.value },
              })
            }
          />
        </label>
        <label>
          Description
          <textarea
            value={draft.profile.description}
            onChange={(event) =>
              setDraft({ ...draft, profile: { ...draft.profile, description: event.target.value } })
            }
          />
        </label>
        <label>
          Avatar art
          <select
            value={draft.profile.avatarRef ?? ''}
            onChange={(event) => setAvatar(event.target.value)}
          >
            <option value="">Default robot</option>
            {avatar.status === 'unavailable' && (
              <option value={avatar.ref}>Unavailable selection · {avatar.ref.slice(0, 22)}…</option>
            )}
            {options.map((option) => (
              <option key={option.ref} value={option.ref}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
        <fieldset
          className="profile-default-options"
          disabled={busy || Boolean(draft.profile.avatarRef)}
        >
          <legend>Default robot appearance</legend>
          {draft.profile.avatarRef && (
            <p>Custom art owns its palette. These saved defaults remain available for fallback.</p>
          )}
          <div className="profile-options">
            {(['hairStyle', 'hairColor', 'skinTone', 'shirtColor'] as const).map((key) => (
              <label key={key}>
                {fieldLabel[key]}
                <select
                  value={draft.profile.appearance[key]}
                  onChange={(event) => setAppearance(key, event.target.value)}
                >
                  {PROFILE_CATALOG[
                    `${key}s` as 'hairStyles' | 'hairColors' | 'skinTones' | 'shirtColors'
                  ].map((value) => (
                    <option key={value}>{value}</option>
                  ))}
                </select>
              </label>
            ))}
          </div>
        </fieldset>
        <label>
          Shirt mark
          <input
            value={draft.profile.appearance.shirtMark}
            onChange={(event) => setAppearance('shirtMark', event.target.value)}
          />
        </label>
        {error && <p role="alert">{error}</p>}
        {storageError && <p role="alert">{storageError}</p>}
        <div className="profile-actions">
          <button disabled={busy} onClick={() => void save()}>
            {busy ? 'Saving…' : 'Save appearance'}
          </button>
          <button
            disabled={busy}
            onClick={() => {
              setDraft({ baseRevision: remote.revision, profile: remote.profile });
              try {
                localStorage.removeItem(draftKey(initial.identityId));
              } catch {
                setStorageError('This draft could not be cleared from this browser.');
              }
              setError(undefined);
            }}
          >
            Load latest profile
          </button>
        </div>
      </fieldset>
    </section>
  );
}
