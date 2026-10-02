import { act, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vite-plus/test';
import { OfficeCanvas } from './office-canvas.js';
import userEvent from '@testing-library/user-event';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import type { OfficeSceneModel } from '../rendering/office-scene.js';

function officeSceneFixture(): OfficeSceneModel {
  return { world: officeWorldFixture().layout, catalog: [], actors: [], components: [] };
}

const scene = vi.hoisted(() => ({
  update: vi.fn(),
  selection: vi.fn(),
  editing: vi.fn(),
  anchorActor: vi.fn(),
  interaction: vi.fn(),
  fit: vi.fn(),
  zoom: vi.fn(),
  dispose: vi.fn(),
}));
const create = vi.hoisted(() => vi.fn());
vi.mock('../rendering/office-scene.js', () => ({
  createOfficeScene: create,
}));
beforeEach(() => {
  vi.clearAllMocks();
  create.mockResolvedValue(scene);
});
afterEach(() => vi.restoreAllMocks());

it('keeps compact camera controls on the scene owner without rebuilding model or drafts', async () => {
  const model = officeSceneFixture();
  const select = vi.fn();
  const view = render(<OfficeCanvas model={model} select={select} />);
  await screen.findByRole('group', { name: 'Map view' });
  expect(screen.getByRole('img', { name: /Drag or scroll to pan; pinch to zoom/ })).toBeTruthy();
  await userEvent.click(screen.getByRole('button', { name: 'Zoom in' }));
  await userEvent.click(screen.getByRole('button', { name: 'Zoom out' }));
  await userEvent.click(screen.getByRole('button', { name: 'Fit office' }));
  expect(scene.zoom.mock.calls).toEqual([[1.2], [1 / 1.2]]);
  expect(scene.fit).toHaveBeenCalledTimes(1);
  expect(scene.update).toHaveBeenCalledExactlyOnceWith(model);
  expect(create).toHaveBeenCalledTimes(1);
  expect(select).not.toHaveBeenCalled();
  view.unmount();
});

it('shares camera-owned meeting bounds and forwards the latest create handler without remounting', async () => {
  const model = officeSceneFixture();
  const previous = vi.fn(),
    createMeeting = vi.fn(),
    select = vi.fn();
  const overlay = vi.fn((target) => (target ? <p>Meeting entry visible</p> : null));
  const view = render(
    <OfficeCanvas model={model} select={select} createMeeting={previous} meetingOverlay={overlay} />
  );
  await waitFor(() => expect(scene.update).toHaveBeenCalled());
  const events = create.mock.calls[0]![2];
  const target = {
    bounds: { x: 600, y: 200, width: 200, height: 180 },
    viewport: { width: 1536, height: 1024 },
  };
  act(() => events.meetingAnchor(target));
  expect(screen.getByText('Meeting entry visible')).toBeTruthy();
  expect(overlay).toHaveBeenLastCalledWith(target);
  view.rerender(
    <OfficeCanvas
      model={model}
      select={select}
      createMeeting={createMeeting}
      meetingOverlay={overlay}
    />
  );
  const slot = { type: 'meeting', index: 2 };
  act(() => events.createMeeting(slot));
  expect(createMeeting).toHaveBeenCalledExactlyOnceWith(slot);
  expect(previous).not.toHaveBeenCalled();
  act(() => events.meetingAnchor(undefined));
  expect(screen.queryByText('Meeting entry visible')).toBeNull();
  expect(create).toHaveBeenCalledTimes(1);
  view.unmount();
  act(() => events.createMeeting(slot));
  expect(createMeeting).toHaveBeenCalledTimes(1);
});

it('docks the same camera controls without recreating the scene or changing its model', async () => {
  const model = officeSceneFixture();
  const select = vi.fn();
  const dock = document.createElement('div');
  document.body.append(dock);
  const view = render(<OfficeCanvas model={model} select={select} />);
  try {
    await screen.findByRole('group', { name: 'Map view' });
    view.rerender(<OfficeCanvas model={model} select={select} cameraHost={dock} />);
    expect(dock.contains(screen.getByRole('group', { name: 'Map view' }))).toBe(true);
    await userEvent.click(screen.getByRole('button', { name: 'Fit office' }));
    view.rerender(<OfficeCanvas model={model} select={select} />);
    expect(dock.childElementCount).toBe(0);
    expect(screen.getAllByRole('group', { name: 'Map view' })).toHaveLength(1);
    expect(create).toHaveBeenCalledTimes(1);
    expect(scene.update).toHaveBeenCalledExactlyOnceWith(model);
    expect(scene.fit).toHaveBeenCalledTimes(1);
    expect(scene.dispose).not.toHaveBeenCalled();
  } finally {
    view.unmount();
    dock.remove();
  }
});

it('renders camera-projected controls with current callbacks and hides them offscreen or outside editing', async () => {
  const model = officeSceneFixture();
  const select = vi.fn();
  const previous = vi.fn(),
    rotate = vi.fn();
  const editor = {
    tool: 'select' as const,
    selected: 'object',
    areaId: 'lobby',
    placeObject: vi.fn(),
  };
  const view = render(
    <OfficeCanvas
      model={model}
      select={select}
      editor={editor}
      furnitureActions={{ label: 'Desk', rotate: previous }}
    />
  );
  await waitFor(() => expect(scene.update).toHaveBeenCalled());
  act(() => create.mock.calls[0]![2].selectedAnchor({ x: 400, y: 300 }));
  expect(
    screen
      .getByRole('group', { name: 'Selected furniture' })
      .style.getPropertyValue('--selection-x')
  ).toBe('400px');
  expect(
    (screen.getByRole('button', { name: 'Move furniture left' }) as HTMLButtonElement).disabled
  ).toBe(true);
  view.rerender(
    <OfficeCanvas
      model={model}
      select={select}
      editor={editor}
      furnitureActions={{ label: 'Desk', rotate }}
    />
  );
  await userEvent.click(screen.getByRole('button', { name: 'Rotate selected furniture' }));
  expect(rotate).toHaveBeenCalledTimes(1);
  expect(previous).not.toHaveBeenCalled();
  act(() => create.mock.calls[0]![2].selectedAnchor(undefined));
  expect(screen.queryByRole('group', { name: 'Selected furniture' })).toBeNull();
  act(() => create.mock.calls[0]![2].selectedAnchor({ x: 400, y: 300 }));
  view.rerender(<OfficeCanvas model={model} select={select} />);
  expect(screen.queryByRole('group', { name: 'Selected furniture' })).toBeNull();
  expect(create).toHaveBeenCalledTimes(1);
  expect(scene.update).toHaveBeenCalledTimes(1);
  view.unmount();
});

it('forwards component focus and the latest activation callback without replacing the scene', async () => {
  const model = officeSceneFixture();
  const previous = vi.fn(),
    activate = vi.fn();
  const select = vi.fn();
  const view = render(<OfficeCanvas model={model} select={select} activate={previous} />);
  await waitFor(() => expect(scene.update).toHaveBeenCalled());
  view.rerender(
    <OfficeCanvas model={model} select={select} activate={activate} focusedComponentId="board" />
  );
  expect(scene.interaction).toHaveBeenLastCalledWith('board');
  act(() => create.mock.calls[0]![2].activate('board'));
  expect(activate).toHaveBeenCalledExactlyOnceWith('board');
  expect(previous).not.toHaveBeenCalled();
  expect(create).toHaveBeenCalledTimes(1);
  expect(scene.update).toHaveBeenCalledTimes(1);
  expect(scene.fit).not.toHaveBeenCalled();
});

it('forwards agent selection without owning a floating inspector or actor anchor', async () => {
  const model = officeSceneFixture();
  const select = vi.fn();
  const target = { kind: 'agent' as const, identityId: 'alice', areaId: 'lobby' };
  const view = render(<OfficeCanvas model={model} select={select} selection={target} />);
  await waitFor(() => expect(scene.selection).toHaveBeenCalledWith(target));
  expect(scene.anchorActor).not.toHaveBeenCalled();
  expect(create.mock.calls[0]![2].actorAnchor).toBeUndefined();
  view.rerender(<OfficeCanvas model={model} select={select} />);
  expect(scene.selection).toHaveBeenLastCalledWith(undefined);
  expect(create).toHaveBeenCalledTimes(1);
  expect(scene.update).toHaveBeenCalledTimes(1);
  expect(select).not.toHaveBeenCalled();
  view.unmount();
});

it('coalesces hidden input changes without rebuilding until visible, then detaches on unmount', async () => {
  let hidden = true;
  vi.spyOn(document, 'hidden', 'get').mockImplementation(() => hidden);
  const select = vi.fn();
  const view = render(<OfficeCanvas model={officeSceneFixture()} select={select} />);
  await waitFor(() => expect(create).toHaveBeenCalledTimes(1));
  view.rerender(
    <OfficeCanvas
      model={officeSceneFixture()}
      select={select}
      selection={{ kind: 'area', areaId: 'old' }}
    />
  );
  const newest = officeSceneFixture();
  view.rerender(
    <OfficeCanvas model={newest} select={select} selection={{ kind: 'area', areaId: 'new' }} />
  );
  expect(scene.update).not.toHaveBeenCalled();
  act(() => {
    hidden = false;
    document.dispatchEvent(new Event('visibilitychange'));
  });
  expect(scene.update).toHaveBeenCalledExactlyOnceWith(newest);
  expect(scene.selection).toHaveBeenCalledExactlyOnceWith({ kind: 'area', areaId: 'new' });
  view.unmount();
  document.dispatchEvent(new Event('visibilitychange'));
  expect(scene.update).toHaveBeenCalledTimes(1);
});

it('does not retain a failed initial scene for later prop updates', async () => {
  scene.update.mockImplementationOnce(() => {
    throw new Error('Raster initialization failed');
  });
  const select = vi.fn();
  const view = render(<OfficeCanvas model={officeSceneFixture()} select={select} />);
  await screen.findByRole('alert');
  expect(scene.update).toHaveBeenCalledTimes(1);
  view.rerender(
    <OfficeCanvas
      model={officeSceneFixture()}
      select={select}
      selection={{ kind: 'area', areaId: 'changed' }}
    />
  );
  expect(scene.update).toHaveBeenCalledTimes(1);
  expect(scene.selection).not.toHaveBeenCalled();
  view.unmount();
});
