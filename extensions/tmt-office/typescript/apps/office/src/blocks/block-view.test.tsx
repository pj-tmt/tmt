import { act, fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import userEvent from '@testing-library/user-event';
import type { Block } from './block-contract.js';
import {
  BUILTIN_DIGEST,
  WORKSHOP_FURNITURE,
  WORKSHOP_DIGEST,
  STUDY_FURNITURE,
  STUDY_DIGEST,
} from '../props/prop-contract.js';
import { builtinFurniture } from './block-contract.js';
import { createBlockState } from './block-state.js';
import { BlockEditor } from './block-view.js';
import type { BlockSceneProps } from './block-scene.js';
import { WORKSHOP_STARTERS, workshopStarter } from './workshop-starter.js';

it('routes spatial furniture commands through the same unsaved draft and removes them while browsing', async () => {
  let scene!: BlockSceneProps;
  const apply = vi.fn(async (_id: string, _revision: number, objects: Block['objects']) => ({
    revision: 3,
    objects,
    updatedAtMs: 3,
  }));
  const original = builtinFurniture('desk', 0, 0, 0);
  const state = createBlockState(
    {
      watch(_id, changed) {
        changed({ revision: 2, objects: [original], updatedAtMs: 2 });
        return () => {};
      },
      apply,
    },
    'room'
  );
  const renderScene = (props: BlockSceneProps) => {
    scene = props;
    return null;
  };
  const view = render(<BlockEditor state={state} renderScene={renderScene} />);
  try {
    act(() => scene.select!(0));
    expect(scene.actions!.left).toBeUndefined();
    expect(scene.actions!.up).toBeUndefined();
    act(() => scene.actions!.right!());
    expect(state.getSnapshot().draft!.objects[0]).toEqual({ ...original, x: 1 });
    expect((screen.getByLabelText('Tile X') as HTMLInputElement).value).toBe('1');
    act(() => scene.actions!.rotate!());
    expect(state.getSnapshot().draft!.objects[0]!.rotation).toBe(1);
    expect(apply).not.toHaveBeenCalled();
    view.rerender(<BlockEditor state={state} renderScene={renderScene} editing={false} />);
    expect(scene.actions).toBeUndefined();
    expect(state.getSnapshot().draft!.objects[0]!.x).toBe(1);
    view.rerender(<BlockEditor state={state} renderScene={renderScene} />);
    act(() => scene.actions!.remove!());
    expect(state.getSnapshot().draft!.objects).toEqual([]);
    expect(scene.actions).toBeUndefined();
    expect(state.getSnapshot().remote!.objects).toEqual([original]);
    await userEvent.click(screen.getByRole('button', { name: 'Save layout' }));
    expect(apply).toHaveBeenCalledExactlyOnceWith('room', 2, []);
  } finally {
    view.unmount();
    state.dispose();
  }
});

it('applies a starter only by explicit action to an empty draft and preserves the revision on save', async () => {
  const apply = vi.fn(async (_id: string, _revision: number, objects: Block['objects']) => ({
    revision: 8,
    objects,
    updatedAtMs: 8,
  }));
  const state = createBlockState(
    {
      watch(_id, changed) {
        changed({ revision: 7, objects: [], updatedAtMs: 7 });
        return () => {};
      },
      apply,
    },
    'room'
  );
  const view = render(
    <BlockEditor state={state} starterLayouts={WORKSHOP_STARTERS} renderScene={() => null} />
  );
  try {
    expect(state.getSnapshot().draft).toBeNull();
    expect(apply).not.toHaveBeenCalled();
    expect((screen.getByRole('combobox', { name: 'Room style' }) as HTMLSelectElement).value).toBe(
      'study'
    );
    await userEvent.click(screen.getByRole('button', { name: 'Try a furnished room' }));
    expect(state.getSnapshot().draft).toEqual({ revision: 7, objects: workshopStarter() });
    expect(state.getSnapshot().remote?.objects).toEqual([]);
    expect(apply).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: 'Try a furnished room' })).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Load latest layout' }));
    expect(state.getSnapshot().draft).toBeNull();
    await userEvent.selectOptions(screen.getByRole('combobox', { name: 'Room style' }), 'library');
    expect(state.getSnapshot().draft).toBeNull();
    expect(apply).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Try a furnished room' }));
    expect(state.getSnapshot().draft).toEqual({ revision: 7, objects: workshopStarter('library') });
    await userEvent.click(screen.getByRole('button', { name: 'Save layout' }));
    expect(apply).toHaveBeenCalledExactlyOnceWith('room', 7, workshopStarter('library'));
    expect(state.getSnapshot().draft).toBeNull();
    expect(screen.queryByRole('button', { name: 'Try a furnished room' })).toBeNull();
    expect(screen.queryByRole('combobox', { name: 'Room style' })).toBeNull();
  } finally {
    view.unmount();
    state.dispose();
  }
});

it('browses without mutation callbacks and preserves selection and an unsaved draft when tools close', async () => {
  let scene!: BlockSceneProps;
  const apply = vi.fn();
  const state = createBlockState(
    {
      watch(_id, changed) {
        changed({ revision: 2, objects: [builtinFurniture('desk', 1, 1, 0)], updatedAtMs: 2 });
        return () => {};
      },
      apply,
    },
    'room'
  );
  const renderScene = (props: BlockSceneProps) => {
    scene = props;
    return <div data-testid="same-scene" />;
  };
  const view = render(<BlockEditor state={state} renderScene={renderScene} editing={false} />);
  try {
    const node = screen.getByTestId('same-scene');
    expect(screen.queryByRole('complementary', { name: 'Furniture controls' })).toBeNull();
    expect(scene.select).toBeUndefined();
    expect(scene.move).toBeUndefined();
    expect(scene.selected).toBeUndefined();
    view.rerender(<BlockEditor state={state} renderScene={renderScene} editing />);
    act(() => scene.select?.(0));
    act(() => scene.move?.(8, 9));
    const draft = state.getSnapshot().draft;
    view.rerender(<BlockEditor state={state} renderScene={renderScene} editing={false} />);
    expect(screen.getByTestId('same-scene')).toBe(node);
    expect(screen.getByRole('status').textContent).toBe('Unsaved changes');
    expect(screen.getByRole('button', { name: 'Save layout' })).toBeTruthy();
    expect(scene.move).toBeUndefined();
    expect(scene.select).toBeUndefined();
    expect(state.getSnapshot().draft).toBe(draft);
    expect(apply).not.toHaveBeenCalled();
    view.rerender(<BlockEditor state={state} renderScene={renderScene} editing />);
    expect(scene.selected).toBe(0);
    expect(scene.objects[0]).toMatchObject({ x: 8, y: 9 });
    view.rerender(<BlockEditor state={state} renderScene={renderScene} editing={false} />);
    await userEvent.click(screen.getByRole('button', { name: 'Load latest layout' }));
    expect(state.getSnapshot().draft).toBeNull();
    expect(scene.objects[0]).toMatchObject({ x: 1, y: 1 });
    expect(screen.queryByRole('button', { name: 'Save layout' })).toBeNull();
  } finally {
    view.unmount();
    state.dispose();
  }
});

it.each([
  {
    pack: undefined,
    button: 'Add desk',
    prop: `${BUILTIN_DIGEST}/desk`,
    footprint: { width: 4, height: 2 },
  },
  {
    pack: WORKSHOP_FURNITURE,
    button: 'Add oak writing desk',
    prop: `${WORKSHOP_DIGEST}/oak-desk`,
    footprint: { width: 12, height: 8 },
  },
  {
    pack: STUDY_FURNITURE,
    button: 'Add oak library bookcase',
    prop: `${STUDY_DIGEST}/oak-bookcase`,
    footprint: { width: 12, height: 12 },
  },
])('uses the same explicit-save draft for $button', async ({ pack, button, prop, footprint }) => {
  const apply = vi.fn(async (_id: string, _revision: number, objects: Block['objects']) => ({
    revision: 1,
    objects,
    updatedAtMs: 1,
  }));
  const state = createBlockState(
    {
      watch(_id, changed) {
        changed(null);
        return () => {};
      },
      apply,
    },
    'room'
  );
  const view = render(
    <BlockEditor
      state={state}
      furniturePacks={pack ? [pack] : undefined}
      renderScene={() => null}
    />
  );
  try {
    const add = screen.getByRole('button', { name: button });
    const preview = add.querySelector('svg');
    expect(preview?.getAttribute('viewBox')).toBe(`0 0 ${footprint.width} ${footprint.height}`);
    expect(preview?.getAttribute('aria-hidden')).toBe('true');
    expect(preview?.querySelector('path')).toBeTruthy();
    await userEvent.click(add);
    const objects = [{ prop, footprint, x: 14, y: 14, rotation: 0 }];
    expect(state.getSnapshot().draft).toEqual({ revision: 0, objects });
    expect(state.getSnapshot().remote).toBeNull();
    expect(apply).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Save layout' }));
    expect(apply).toHaveBeenCalledExactlyOnceWith('room', 0, objects);
    expect(state.getSnapshot().draft).toBeNull();
  } finally {
    view.unmount();
    state.dispose();
  }
});

it('keeps renderer interactions in the same revision-aware draft until explicit save', async () => {
  let scene!: BlockSceneProps;
  const apply = vi.fn(async (_id: string, _revision: number, objects: Block['objects']) => ({
    revision: 5,
    objects,
    updatedAtMs: 5,
  }));
  const state = createBlockState(
    {
      watch(_id, changed) {
        changed({ revision: 4, objects: [builtinFurniture('desk', 1, 1, 0)], updatedAtMs: 4 });
        return () => {};
      },
      apply,
    },
    'room'
  );
  const view = render(
    <BlockEditor
      state={state}
      renderScene={(props) => {
        scene = props;
        return null;
      }}
    />
  );
  try {
    const catalog = scene.catalog;
    const initialObjects = scene.objects;
    act(() => scene.select?.(0));
    expect(scene.catalog).toBe(catalog);
    expect(scene.objects).toBe(initialObjects);
    act(() => scene.move?.(8, 9));
    expect(scene.objects[0]).toMatchObject({ x: 8, y: 9 });
    expect(state.getSnapshot().remote?.objects[0]).toMatchObject({ x: 1, y: 1 });
    expect(apply).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Load latest layout' }));
    expect(scene.objects[0]).toMatchObject({ x: 1, y: 1 });
    expect(scene.selected).toBeNull();
    act(() => scene.select?.(0));
    act(() => scene.move?.(6, 7));
    await userEvent.click(screen.getByRole('button', { name: 'Save layout' }));
    expect(apply).toHaveBeenCalledExactlyOnceWith('room', 4, [builtinFurniture('desk', 6, 7, 0)]);
    expect(state.getSnapshot().draft).toBeNull();
  } finally {
    view.unmount();
    state.dispose();
  }
});

it('edits declared rug customization through the same discard and explicit-save draft', async () => {
  const rug = {
    prop: `${WORKSHOP_DIGEST}/woven-rug`,
    footprint: { width: 16, height: 12 },
    x: 8,
    y: 8,
    rotation: 0,
  };
  const apply = vi.fn(async (_id: string, revision: number, objects: Block['objects']) => ({
    revision: revision + 1,
    objects,
    updatedAtMs: 1,
  }));
  const state = createBlockState(
    {
      watch(_id, changed) {
        changed({ revision: 1, objects: [rug], updatedAtMs: 1 });
        return () => {};
      },
      apply,
    },
    'room'
  );
  const view = render(
    <BlockEditor
      state={state}
      furniturePacks={[WORKSHOP_FURNITURE, STUDY_FURNITURE]}
      renderScene={() => null}
    />
  );
  try {
    await userEvent.click(screen.getByRole('button', { name: 'Woven workshop rug 1' }));
    fireEvent.change(screen.getByLabelText('Tint color'), { target: { value: '#803060' } });
    await userEvent.type(screen.getByLabelText('Display text'), 'Studio');
    expect(state.getSnapshot().draft?.objects[0]).toEqual({
      ...rug,
      customization: { tint: '#803060', text: 'Studio' },
    });
    expect(state.getSnapshot().remote?.objects).toEqual([rug]);
    expect(apply).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Load latest layout' }));
    await userEvent.click(screen.getByRole('button', { name: 'Woven workshop rug 1' }));
    expect(screen.getByLabelText('Display text')).toHaveProperty('value', '');
    expect(screen.getByRole('button', { name: 'Restore original color' })).toHaveProperty(
      'disabled',
      true
    );
    fireEvent.change(screen.getByLabelText('Tint color'), { target: { value: '#803060' } });
    await userEvent.type(screen.getByLabelText('Display text'), 'Studio');
    await userEvent.click(screen.getByRole('button', { name: 'Save layout' }));
    expect(apply).toHaveBeenCalledExactlyOnceWith('room', 1, [
      { ...rug, customization: { tint: '#803060', text: 'Studio' } },
    ]);
    await userEvent.click(screen.getByRole('button', { name: 'Restore original color' }));
    await userEvent.clear(screen.getByLabelText('Display text'));
    expect(state.getSnapshot().draft?.objects).toEqual([rug]);
    expect(state.getSnapshot().draft?.objects[0]).not.toHaveProperty('customization');
    await userEvent.click(screen.getByRole('button', { name: 'Save layout' }));
    expect(apply).toHaveBeenLastCalledWith('room', 2, [rug]);
  } finally {
    view.unmount();
    state.dispose();
  }
});

it('distinguishes connecting, absent, saved and unavailable block observations', () => {
  let changed: (block: Block | null) => void = () => {};
  let failed = () => {};
  const state = createBlockState(
    {
      watch: (_id, next, error) => {
        changed = next;
        failed = error;
        return () => {};
      },
      apply: async () => {
        throw new Error('This read-only scenario must not write.');
      },
    },
    'a'.repeat(20)
  );
  const view = render(<BlockEditor state={state} />);
  try {
    expect(screen.getByRole('status').textContent).toBe('Connecting…');
    act(() => changed(null));
    expect(screen.getByRole('status').textContent).toBe('No saved layout yet');
    act(() => changed({ revision: 1, objects: [], updatedAtMs: 1 }));
    expect(screen.getByRole('status').textContent).toBe('Saved · revision 1');
    act(failed);
    expect(screen.getByRole('status').textContent).toBe('Block unavailable');
    expect(screen.getByRole('alert').textContent).toContain('Reopen the world');
    expect(screen.queryByRole('button', { name: 'Save layout' })).toBeNull();
  } finally {
    view.unmount();
    state.dispose();
  }
});

it('keeps mismatched geometry as a bounded placeholder without dropping its neighbor', () => {
  let changed: (block: Block | null) => void = () => {};
  const state = createBlockState(
    {
      watch: (_id, next) => {
        changed = next;
        return () => {};
      },
      apply: async () => {
        throw new Error('This read-only scenario must not write.');
      },
    },
    'a'.repeat(20)
  );
  const objects = [
    { ...builtinFurniture('desk', 1, 1, 0), footprint: { width: 1, height: 1 } },
    builtinFurniture('chair', 4, 1, 0),
  ];
  const view = render(<BlockEditor state={state} />);
  try {
    act(() => changed({ revision: 7, objects, updatedAtMs: 7 }));
    expect(screen.getByRole('status').textContent).toBe('Saved · revision 7');
    expect(screen.getByRole('button', { name: 'Unavailable prop 1' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Chair 2' })).toBeTruthy();
    expect(
      screen.getByRole('img', { name: `Unavailable prop ${BUILTIN_DIGEST.slice(7, 19)}` })
    ).toBeTruthy();
    const scene = screen.getByRole('group', { name: /^Office layout, 32 by 32 tiles/ });
    expect(scene.querySelector('path[fill="#dca77dff"]')).toBeTruthy();
    expect(state.getSnapshot().remote).toMatchObject({ revision: 7, objects });
  } finally {
    view.unmount();
    state.dispose();
  }
});
