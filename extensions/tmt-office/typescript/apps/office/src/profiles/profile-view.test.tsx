import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { PROFILE_CATALOG, ProfileConflict } from './profile-contract.js';
import type { Profile, ProfileMutation, ProfilePort, ProfileSnapshot } from './profile-contract.js';
import { ProfilePanel } from './profile-view.js';
import { decodeAvatarCatalog } from '../avatars/avatar-catalog.js';
import avatarVectors from '../../../../../contracts/avatar-pack-vectors.json';

const avatarDigest = `sha256:${'1'.repeat(64)}`;
const avatarRef = `${avatarDigest}/signal-bot`;
const avatarCatalog = decodeAvatarCatalog({
  catalogRevision: 1,
  packs: [{ digest: avatarDigest, pack: avatarVectors.packCases[0]!.value }],
});

const snapshot: ProfileSnapshot = {
  identityId: '22222222-2222-4222-8222-222222222222',
  identityName: '<img onerror=alert(1)>',
  exists: true,
  revision: 2,
  profile: {
    displayLabel: '',
    description: '',
    appearance: {
      hairStyle: 'curls',
      hairColor: 'silver',
      skinTone: 'deep',
      shirtColor: 'plum',
      shirtMark: '<b>',
    },
  },
  updatedAtMs: 2,
  catalog: PROFILE_CATALOG,
};

beforeEach(() => {
  const values = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
    clear: () => values.clear(),
  });
});
afterEach(() => vi.unstubAllGlobals());

function mutation(value: ProfileSnapshot, changed = false): ProfileMutation {
  return { ...value, changed };
}

it('keeps the original draft revision through conflict, reread and remount', async () => {
  const apply = vi.fn<ProfilePort['apply']>(async () => {
    throw new ProfileConflict();
  });
  const port: ProfilePort = {
    list: async () => [],
    show: vi.fn(async () => ({ ...snapshot, revision: 3 })),
    apply,
  };
  const changed = vi.fn();
  const first = render(<ProfilePanel initial={snapshot} port={port} changed={changed} />);
  expect(document.querySelector('.avatar-name')?.textContent).toBe('<img onerror=alert(1)>');
  expect(document.querySelector('img')).toBeNull();
  fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'Unsaved draft' } });
  fireEvent.click(screen.getByRole('button', { name: 'Save appearance' }));
  await screen.findByRole('alert');
  expect((screen.getByLabelText('Description') as HTMLTextAreaElement).value).toBe('Unsaved draft');
  await waitFor(() => expect(port.show).toHaveBeenCalled());
  fireEvent.click(screen.getByRole('button', { name: 'Save appearance' }));
  await waitFor(() => expect(apply).toHaveBeenCalledTimes(2));
  expect(apply.mock.calls.map((call) => call[1])).toEqual([2, 2]);
  first.unmount();
  render(<ProfilePanel initial={{ ...snapshot, revision: 3 }} port={port} changed={changed} />);
  expect((screen.getByLabelText('Description') as HTMLTextAreaElement).value).toBe('Unsaved draft');
  fireEvent.click(screen.getByRole('button', { name: 'Save appearance' }));
  await waitFor(() => expect(apply).toHaveBeenCalledTimes(3));
  expect(apply.mock.calls[2]?.[1]).toBe(2);
});

it('retains bounded invalid in-progress text across remount', () => {
  const port: ProfilePort = {
    list: async () => [],
    show: async () => snapshot,
    apply: async () => mutation(snapshot),
  };
  const first = render(<ProfilePanel initial={snapshot} port={port} changed={() => undefined} />);
  const text = 'x'.repeat(1025);
  fireEvent.change(screen.getByLabelText('Description'), { target: { value: text } });
  first.unmount();
  render(<ProfilePanel initial={snapshot} port={port} changed={() => undefined} />);
  expect((screen.getByLabelText('Description') as HTMLTextAreaElement).value).toBe(text);
});

it('retains an uncertain intent for an exact retry at its original revision', async () => {
  const draft: Profile = { ...snapshot.profile, description: 'Committed but unconfirmed' };
  const committed = { ...snapshot, revision: 3, profile: draft, updatedAtMs: 3 };
  const apply = vi
    .fn<ProfilePort['apply']>()
    .mockRejectedValueOnce(new Error('connection lost'))
    .mockResolvedValueOnce(mutation(committed));
  const port: ProfilePort = { list: async () => [], show: async () => committed, apply };
  render(<ProfilePanel initial={snapshot} port={port} changed={() => undefined} />);
  fireEvent.change(screen.getByLabelText('Description'), { target: { value: draft.description } });
  fireEvent.click(screen.getByRole('button', { name: 'Save appearance' }));
  await screen.findByText('Saved · revision 3');
  fireEvent.click(screen.getByRole('button', { name: 'Save appearance' }));
  await waitFor(() => expect(apply).toHaveBeenCalledTimes(2));
  expect(apply.mock.calls.map((call) => call[1])).toEqual([2, 2]);
});

it('disables every field while saving and catches draft storage failure', async () => {
  let finish: ((value: ProfileMutation) => void) | undefined;
  const apply = vi.fn(
    () =>
      new Promise<ProfileMutation>((resolve) => {
        finish = resolve;
      })
  );
  const port: ProfilePort = { list: async () => [], show: async () => snapshot, apply };
  const broken = localStorage as unknown as { setItem: (key: string, value: string) => void };
  broken.setItem = () => {
    throw new DOMException('quota', 'QuotaExceededError');
  };
  render(<ProfilePanel initial={snapshot} port={port} changed={() => undefined} />);
  fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'Keep me' } });
  await screen.findByText(/could not be saved in this browser/);
  fireEvent.click(screen.getByRole('button', { name: 'Save appearance' }));
  expect(
    (screen.getByLabelText('Description').closest('fieldset') as HTMLFieldSetElement).disabled
  ).toBe(true);
  finish?.(
    mutation({ ...snapshot, profile: { ...snapshot.profile, description: 'Keep me' } }, true)
  );
  await waitFor(() =>
    expect(
      (screen.getByLabelText('Description').closest('fieldset') as HTMLFieldSetElement).disabled
    ).toBe(false)
  );
});

it('keeps full accessible text in bounded labels without distorting glyphs', () => {
  const displayLabel = 'Architecture '.repeat(7).slice(0, 80);
  const marked = {
    ...snapshot,
    identityName: '\u8a2d\u8a08\u5718\u968a 🚀',
    profile: {
      ...snapshot.profile,
      displayLabel,
      appearance: { ...snapshot.profile.appearance, shirtMark: '🚀🚀' },
    },
  };
  const port: ProfilePort = {
    list: async () => [],
    show: async () => marked,
    apply: async () => mutation(marked),
  };
  render(<ProfilePanel initial={marked} port={port} changed={() => undefined} />);
  expect(screen.getByText(displayLabel).parentElement?.getAttribute('width')).toBe('12');
  expect(screen.getByText(displayLabel).getAttribute('title')).toBe(displayLabel);
  expect(document.querySelector('.avatar-name')?.getAttribute('title')).toBe(
    '\u8a2d\u8a08\u5718\u968a 🚀'
  );
  expect(screen.getByText('🚀🚀').parentElement?.getAttribute('width')).toBe('4');
  expect(document.querySelector('[textLength], [lengthAdjust]')).toBeNull();
  expect(document.querySelector('img')).toBeNull();
  expect(document.querySelector('.profile-preview title')?.textContent).toContain(displayLabel);
  expect(document.querySelector('.profile-preview title')?.textContent).toContain(
    '\u8a2d\u8a08\u5718\u968a 🚀'
  );
});

it('uses custom art while retaining default controls and the independent shirt mark', async () => {
  const selected = {
    ...snapshot,
    profile: { ...snapshot.profile, avatarRef },
  };
  const apply = vi.fn<ProfilePort['apply']>(async (_id, _revision, profile) =>
    mutation({ ...snapshot, revision: 3, profile }, true)
  );
  const port: ProfilePort = { list: async () => [], show: async () => selected, apply };
  render(
    <ProfilePanel
      initial={selected}
      port={port}
      avatarCatalog={avatarCatalog}
      changed={() => undefined}
    />
  );
  expect(screen.getByText('Avatar · Signal bots · Signal bot')).toBeTruthy();
  expect(screen.getByLabelText('Hair style').matches(':disabled')).toBe(true);
  expect(screen.getByLabelText('Shirt mark').matches(':disabled')).toBe(false);
  expect(document.querySelector('.profile-preview path')?.getAttribute('fill')).toBe('#ffffffff');

  fireEvent.change(screen.getByLabelText('Avatar art'), { target: { value: '' } });
  expect(screen.getByLabelText('Hair style').matches(':disabled')).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'Save appearance' }));
  await waitFor(() => expect(apply).toHaveBeenCalled());
  const saved = apply.mock.calls[0]?.[2];
  expect(saved).not.toHaveProperty('avatarRef');
  expect(saved?.appearance).toEqual(snapshot.profile.appearance);
});

it('retains an unavailable selection and renders the saved default fallback', () => {
  const unavailable = {
    ...snapshot,
    profile: { ...snapshot.profile, avatarRef: `${avatarDigest}/missing` },
  };
  const port: ProfilePort = {
    list: async () => [],
    show: async () => unavailable,
    apply: async () => mutation(unavailable),
  };
  render(
    <ProfilePanel
      initial={unavailable}
      port={port}
      avatarCatalog={avatarCatalog}
      changed={() => undefined}
    />
  );
  expect(screen.getByText('Avatar · unavailable, showing saved default appearance')).toBeTruthy();
  expect((screen.getByLabelText('Avatar art') as HTMLSelectElement).value).toBe(
    `${avatarDigest}/missing`
  );
  expect(document.querySelector('.profile-preview path')?.getAttribute('fill')).not.toBe(
    '#ffffffff'
  );
});
