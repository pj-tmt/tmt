import { Container, Graphics, Sprite, Texture } from 'pixi.js';
import { afterEach, expect, it, vi } from 'vitest';
import { createOfficeScene } from './office-scene.js';
import type { OfficeSceneModel } from './office-scene.js';
import { DIRECTIONAL_WORKSTATION_DIGEST } from '../props/prop-contract.js';

// Only browser/GPU resources and text rasterization are stubbed. Selection,
// geometry, display-list ordering, prop rendering and actor placement are real.
let stage: Container;
const lobby = '10000000-0000-4000-8000-000000000001';
const office = '10000000-0000-4000-8000-000000000002';
const meeting = '10000000-0000-4000-8000-000000000003';
afterEach(() => vi.unstubAllGlobals());
vi.mock('./scene-application.js', () => ({
  createSceneApplication: async () => {
    stage = new Container();
    return {
      application: {
        stage,
        canvas: document.createElement('canvas'),
        renderer: { on() {}, off() {} },
      },
      invalidate() {},
      dispose() {
        stage.destroy({ children: true });
      },
    };
  },
}));
vi.mock('./scene-materials.js', () => ({
  createSceneMaterials: async () => ({
    forMaterial: () => ({ floor: Texture.EMPTY }),
    dispose() {},
  }),
}));
vi.mock('./platform-art.js', async (original) => ({
  ...(await original<typeof import('./platform-art.js')>()),
  createPlatformArt: async () => ({
    textures: new Proxy({}, { get: () => Texture.EMPTY }),
    dispose() {},
  }),
}));
vi.mock('./scene-textures.js', () => ({
  createSceneTextures: () => ({ begin() {}, end() {}, dispose() {}, get: () => Texture.EMPTY }),
}));
vi.mock('./scene-label.js', () => ({
  sceneLabel: (parent: Container) => parent.addChild(new Sprite(Texture.EMPTY)),
}));
vi.mock('./scene-nameplate.js', () => ({
  sceneNameplate: (parent: Container) => {
    parent.addChild(new Container({ label: 'area-name' }));
    return { zoom() {} };
  },
  sceneActorLabel: (parent: Container) => {
    parent.addChild(new Container({ label: 'actor-name' }));
    return { zoom() {} };
  },
}));

it('keeps typed area and exact-instance actor selection below upright content while object handles stay above it', async () => {
  vi.stubGlobal('matchMedia', () => ({
    matches: true,
    addEventListener() {},
    removeEventListener() {},
  }));
  const host = document.createElement('div');
  Object.defineProperties(host, { clientWidth: { value: 1440 }, clientHeight: { value: 1000 } });
  const model: OfficeSceneModel = {
    world: {
      version: 1,
      map: {
        version: 8,
        primaryLobbyId: lobby,
        modules: [
          {
            area: { id: lobby, name: 'Lobby', binding: { type: 'lobby' } },
            slot: { type: 'lobby' },
            material: 'workshop',
          },
          {
            area: { id: office, name: 'Office', binding: { type: 'personal', identityId: null } },
            slot: { type: 'office', column: 0, row: -1 },
            material: 'workshop',
          },
          {
            area: {
              id: meeting,
              name: 'Meeting',
              binding: { type: 'meeting', roomId: '40000000-0000-4000-8000-000000000001' },
            },
            slot: { type: 'office', column: 1, row: -1 },
            material: 'workshop',
          },
        ],
      },
      objects: [
        {
          id: 'bookcase',
          kind: 'decoration',
          surface: { type: 'floor' },
          placement: {
            prop: `${DIRECTIONAL_WORKSTATION_DIGEST}/workstation-bookcase`,
            x: 10,
            y: 5,
            rotation: 0,
            footprint: { width: 11, height: 11 },
          },
          extension: null,
        },
      ],
    },
    actors: [office, meeting].map((areaId) => ({
      identityId: 'alice',
      areaId,
      contractor: false,
      avatar: {
        name: 'Alice',
        appearance: {
          hairStyle: 'short',
          hairColor: 'ink',
          skinTone: 'light',
          shirtColor: 'blue',
          shirtMark: 'AI',
        },
      },
    })),
    catalog: [],
    components: [],
  };
  const scene = (await createOfficeScene(host, new AbortController().signal, { select() {} }))!;
  try {
    scene.update(model);
    const root = stage.children[1] as Container;
    const ground = root.getChildByLabel('ground-selection') as Graphics;
    const foreground = root.getChildByLabel('object-selection') as Graphics;
    const poly = vi.spyOn(ground, 'poly');
    const ellipse = vi.spyOn(ground, 'ellipse');
    const stroke = vi.spyOn(ground, 'stroke');
    for (const [areaId, color] of [
      [lobby, '#ffe2a0'],
      [office, '#9bffe3'],
      [meeting, '#d8b0ff'],
    ]) {
      scene.selection({ kind: 'area', areaId: areaId! });
      expect(stroke).toHaveBeenLastCalledWith({ color, width: 0.3 });
    }
    const groundIndex = root.getChildIndex(ground);
    const labels = root.children.filter(
      (node) =>
        node.label === 'area-name' || node.children.some((child) => child.label === 'actor-name')
    );
    expect(labels.length).toBeGreaterThan(0);
    for (const label of labels) expect(root.getChildIndex(label)).toBeGreaterThan(groundIndex);
    // All sprite-bearing upright groups follow the ground highlight. Floor tiles
    // are direct children, whereas props and actors are grouped depth-sorted nodes.
    const upright = root.children.filter(
      (node) => node.label === 'world-object' || node.label === 'world-actor'
    );
    expect(upright.length).toBeGreaterThanOrEqual(3);
    for (const node of upright) {
      expect(root.getChildIndex(node)).toBeGreaterThan(groundIndex);
      expect(root.getChildIndex(node)).toBeLessThan(root.getChildIndex(foreground));
    }
    poly.mockClear();
    scene.selection({ kind: 'agent', identityId: 'alice', areaId: office });
    expect(poly).not.toHaveBeenCalled();
    expect(ellipse).toHaveBeenCalledTimes(1);
    const first = ellipse.mock.calls[0];
    scene.selection({ kind: 'agent', identityId: 'alice', areaId: meeting });
    expect(ellipse).toHaveBeenCalledTimes(2);
    expect(ellipse.mock.calls[1]).not.toEqual(first);
    scene.selection(undefined);
    expect(ground.context.instructions).toHaveLength(0);
    scene.editing({ areaId: office, selected: 'bookcase', placeObject() {} });
    expect(foreground.context.instructions.length).toBeGreaterThan(0);
    scene.zoom(1.1);
    expect(ground.destroyed).toBe(false);
    expect(root.getChildIndex(ground)).toBeLessThan(root.getChildIndex(foreground));
  } finally {
    scene.dispose();
  }
});
