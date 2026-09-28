import { Container, Graphics, Sprite } from 'pixi.js';
import type { SceneAvatar } from '../blocks/block-scene.js';
import type { CatalogPack } from '../props/prop-contract.js';
import type { WorldDocument, WorldObject } from '../world-map/world-contract.js';
import { resolvePlacedProp } from '../props/prop-contract.js';
import { canRotateObject, rotatedObject } from '../world-map/object-rotation.js';
import { withFurnitureBase } from '../world-map/furniture-base.js';
import { floorObjectBounds } from '../world-map/object-base.js';
import type { CatalogPlacement } from './catalog-placement.js';
import { footprint } from '../blocks/block-contract.js';
import { avatarArt } from '../profiles/avatar-art.js';
import { AVATAR_LAYOUT, avatarMarkColor, avatarTopInset } from '../profiles/avatar-layout.js';
import { createSceneApplication } from './scene-application.js';
import { createSceneMaterials } from './scene-materials.js';
import { createPlatformArt } from './platform-art.js';
import { drawPlatformEdge, platformContour } from './scene-platform.js';
import { moduleBounds } from '../world-map/module-geometry.js';
import { createSceneTextures } from './scene-textures.js';
import { createSceneFloor } from './scene-floor.js';
import { drawBridgeBase, drawBridgeDeck } from './scene-skybridge.js';
import { createBridgePulse } from './bridge-pulse.js';
import { drawWall } from './scene-wall.js';
import { drawModuleGhost, moduleGhostGeometry } from './scene-module-ghost.js';
import { officeExpansionPassages } from '../world-map/module-geometry.js';
import { officeSlotKey } from '../world-map/module-contract.js';
import type { MeetingSlot, OfficeSlot } from '../world-map/module-contract.js';
import { mapGeometry } from '../world-map/map-source.js';
import { placementProblem } from '../world-map/world-object-placement.js';
import { sceneLabel } from './scene-label.js';
import { sceneActorLabel, sceneNameplate } from './scene-nameplate.js';
import { drawSceneProp } from './scene-props.js';
import { drawSceneComponents } from './scene-components.js';
import type { SceneAction, SceneComponent } from './scene-component-geometry.js';
import { selectionAnchor, selectionTarget } from './selection-anchor.js';
import type { SelectionAnchor, SelectionTarget } from './selection-anchor.js';
import type { OfficeSelection } from './office-selection.js';
import {
  fitOfficeCamera,
  resizeOfficeCamera,
  scenePoint,
  zoomOfficeCamera,
  wheelOfficeCamera,
} from './office-geometry.js';
import type { SceneRect } from './office-geometry.js';
import { flatProjection } from './world-projection.js';
import { intersects, wallProjection, worldGeometry } from './world-geometry.js';

export interface OfficeActor {
  identityId: string;
  areaId: string;
  avatar: SceneAvatar;
  contractor: boolean;
  activity?: string;
}
export interface OfficeSceneModel {
  world: WorldDocument;
  actors: OfficeActor[];
  catalog: CatalogPack[];
  components: SceneAction[];
}
export interface OfficeSceneEditor {
  areaId: string;
  selectedMeeting?: MeetingSlot;
  clearSelection?: () => void;
  officeSlots?: readonly OfficeSlot[];
  selectedOffice?: OfficeSlot;
  chooseOffice?: (slot: OfficeSlot) => void;
  selected?: string;
  select?: (id: string) => void;
  placeObject: (object: WorldObject) => void;
}

export interface OfficeSceneEvents {
  select(selection: OfficeSelection): void;
  activate?(id: string): void;
  selectedAnchor?(anchor: SelectionAnchor | undefined): void;
  officeAnchor?(target: SelectionTarget | undefined): void;
  meetingAnchor?(target: SelectionTarget | undefined): void;
  createMeeting?(slot: MeetingSlot): void;
}

/** Pixels and camera only. Every room, prop and door projects the world draft. */
export async function createOfficeScene(
  host: HTMLElement,
  signal: AbortSignal,
  {
    select,
    activate = () => {},
    selectedAnchor = () => {},
    officeAnchor = () => {},
    meetingAnchor = () => {},
    createMeeting = () => {},
  }: OfficeSceneEvents
) {
  const runtime = await createSceneApplication(host, signal);
  if (!runtime) return undefined;
  const { application, invalidate, dispose: disposeApplication } = runtime;
  const root = new Container(),
    backdrop = new Graphics();
  application.stage.addChild(backdrop, root);
  const textures = createSceneTextures();
  let materials: Awaited<ReturnType<typeof createSceneMaterials>>;
  let platformArt: Awaited<ReturnType<typeof createPlatformArt>>;
  try {
    materials = await createSceneMaterials(signal);
    platformArt = await createPlatformArt(signal);
  } catch (error) {
    materials?.dispose();
    platformArt?.dispose();
    disposeApplication();
    if (signal.aborted) return undefined;
    throw error;
  }
  if (!materials || !platformArt) {
    materials?.dispose();
    platformArt?.dispose();
    disposeApplication();
    return undefined;
  }
  let model: OfficeSceneModel | undefined;
  let geometry: ReturnType<typeof worldGeometry> | undefined;
  const unprojectGround = (point: { x: number; y: number }) =>
    (geometry?.projection ?? flatProjection).unprojectGround(point);
  const projectGroundRect = (rect: SceneRect) =>
    (geometry?.projection ?? flatProjection).projectGroundRect(rect);
  const projectUpright = (rect: SceneRect) =>
    (geometry?.projection ?? flatProjection).projectUpright(rect);
  let camera = { x: 0, y: 0, scale: 1 };
  function cameraFrame() {
    return {
      x: 0,
      y: 0,
      width: Math.max(1, host.clientWidth),
      height: Math.max(1, host.clientHeight),
    };
  }
  let cameraViewport = cameraFrame();
  let fitted = false,
    disposed = false;
  let editor: OfficeSceneEditor | undefined;
  let objectPickOrder: OfficeSceneModel['world']['objects'] = [];
  let selected: OfficeSelection | undefined;
  let focused: string | undefined, hovered: string | undefined;
  let componentLayer: ReturnType<typeof drawSceneComponents> | undefined;
  let nameplates: ReturnType<typeof sceneNameplate>[] = [];
  let actors: { actor: OfficeActor; bounds: SceneRect }[] = [];
  let renderedView: SceneRect | undefined;
  const objectLayers = new Map<string, Container>();
  const bridgePulse = createBridgePulse(invalidate);
  let bridgeLights: { light: Graphics; bounds: SceneRect }[] = [];
  function refreshBridgeLights() {
    const view = viewport();
    bridgePulse.setLights(
      bridgeLights.filter(({ bounds }) => intersects(view, bounds)).map(({ light }) => light)
    );
  }
  const preview = new Graphics(),
    groundHighlight = new Graphics(),
    highlight = new Graphics();
  const catalogGhost = new Container();
  groundHighlight.label = 'ground-selection';
  highlight.label = 'object-selection';
  const catalogTextures = createSceneTextures();
  const canvas = application.canvas;
  canvas.style.touchAction = 'none';
  canvas.tabIndex = -1;
  function viewport() {
    return {
      ...scenePoint(camera, 0, 0),
      width: Math.max(1, host.clientWidth) / camera.scale,
      height: Math.max(1, host.clientHeight) / camera.scale,
    };
  }
  function interaction(id?: string) {
    focused = id;
    if (componentLayer?.highlight(hovered ?? focused)) invalidate();
  }
  function selection(value?: OfficeSelection) {
    selected = value;
    groundHighlight.clear();
    highlight.clear();
    if (!model || !geometry) return;
    const view = viewport();
    const area =
      value?.kind === 'area'
        ? mapGeometry(model.world.map).areas.find((area) => area.id === value.areaId)
        : undefined;
    const color =
      area?.binding.type === 'meeting'
        ? '#d8b0ff'
        : area?.binding.type === 'lobby'
          ? '#ffe2a0'
          : '#9bffe3';
    if (area && model.world.map.version >= 6 && model.world.map.version !== 1) {
      const module = model.world.map.modules.find((item) => item.area.id === value?.areaId);
      if (module) {
        const contour = platformContour(
          geometry.projection.projectModuleFloor(moduleBounds(module, model.world.map.version))
        );
        groundHighlight.poly(contour).fill({ color, alpha: 0.035 });
        groundHighlight.poly(contour).stroke({ color, width: 1.1, alpha: 0.15 });
        groundHighlight.poly(contour).stroke({ color, width: 0.3 });
      }
    }
    for (const floor of area ? geometry.visible(view).floors : [])
      if (model.world.map.version < 6 && floor.areaId === value?.areaId && intersects(view, floor))
        groundHighlight
          .rect(floor.x, floor.y, floor.width, floor.height)
          .fill({ color, alpha: 0.12 });
    if (value?.kind === 'agent') {
      const instance = actors.find(
        ({ actor }) => actor.identityId === value.identityId && actor.areaId === value.areaId
      );
      if (instance) {
        const bounds = instance.bounds;
        groundHighlight
          .ellipse(bounds.x + bounds.width / 2, bounds.y + bounds.height, 3.2, 0.9)
          .fill({ color: '#9bffe3', alpha: 0.15 })
          .stroke({ color: '#9bffe3', width: 0.25 });
      }
    }
    const object =
      pointer?.rotationPreview ?? model.world.objects.find((item) => item.id === editor?.selected);
    const rect = object && geometry.objectRect(object);
    if (rect)
      highlight
        .rect(rect.x, rect.y, rect.width, rect.height)
        .stroke({ color: '#ffe8a2', width: 0.25 });
    for (const corner of rotationCorners(object)) {
      highlight
        .circle(corner.x, corner.y, 5 / camera.scale)
        .fill({ color: '#102b35' })
        .stroke({ color: '#9bffe3', width: 2 / camera.scale });
    }
    selectedAnchor(
      rect
        ? selectionAnchor(rect, camera, { width: host.clientWidth, height: host.clientHeight })
        : undefined
    );
    officeAnchor(
      editor?.selectedOffice
        ? selectionTarget(
            moduleGhostGeometry(editor.selectedOffice, geometry.projection).bounds,
            camera,
            {
              width: host.clientWidth,
              height: host.clientHeight,
            }
          )
        : undefined
    );
    meetingAnchor(
      geometry.meetingPreview
        ? selectionTarget(
            moduleGhostGeometry(geometry.meetingPreview.slot, geometry.projection).bounds,
            camera,
            { width: host.clientWidth, height: host.clientHeight }
          )
        : undefined
    );
    invalidate();
  }
  function sky() {
    const width = Math.max(1, host.clientWidth),
      height = Math.max(1, host.clientHeight);
    backdrop.clear().rect(0, 0, width, height).fill('#040d20');
    // Fixed-cost distant haze stays in this single screen-space backdrop.
    // Layered translucent contours need neither blur textures nor a ticker.
    for (const [cx, cy, rx, ry, color] of [
      [0.08, 0.28, 0.42, 0.34, '#184269'],
      [0.91, 0.7, 0.36, 0.42, '#183853'],
      [0.53, 0.03, 0.28, 0.24, '#262947'],
    ] as const) {
      for (let ring = 24; ring > 0; ring--) {
        const radius = ring / 24;
        backdrop
          .ellipse(width * cx, height * cy, width * rx * radius, height * ry * radius)
          .fill({ color, alpha: 0.018 });
      }
    }
    // Fixed seed and bounded screen density: distant dust, luminous stars and
    // sparse diffraction crosses. No ticker, blur filter or per-star display node.
    let seed = 291;
    const random = () => {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      return seed / 4294967296;
    };
    const count = Math.min(640, Math.max(80, Math.round((width * height) / 3000)));
    for (let i = 0; i < count; i++) {
      const x = Math.floor(random() * width),
        y = Math.floor(random() * height);
      const bright = i % 17 === 0,
        middle = i % 4 === 0;
      if (bright) {
        backdrop.circle(x, y, 12).fill({ color: '#3877d8', alpha: 0.035 });
        backdrop.circle(x, y, 7).fill({ color: '#559cfa', alpha: 0.08 });
        backdrop.circle(x, y, 3).fill({ color: '#82beff', alpha: 0.2 });
        backdrop.rect(x - 5, y, 11, 1).fill({ color: '#89c8ff', alpha: 0.5 });
        backdrop.rect(x, y - 7, 1, 15).fill({ color: '#89c8ff', alpha: 0.5 });
      }
      backdrop.rect(x, y, bright ? 2 : 1, bright ? 2 : 1).fill({
        color: bright ? '#e9f5ff' : middle ? '#a6caff' : '#5780b7',
        alpha: bright ? 0.95 : middle ? 0.65 : 0.35,
      });
    }
  }
  function draw() {
    if (!model || !geometry || !materials) return;
    bridgePulse.setLights([]);
    bridgeLights = [];
    root.removeChild(preview, groundHighlight, highlight, catalogGhost);
    for (const child of root.removeChildren()) child.destroy({ children: true });
    objectLayers.clear();
    textures.begin();
    nameplates = [];
    actors = [];
    const visible = viewport();
    renderedView = {
      x: visible.x - 32,
      y: visible.y - 32,
      width: visible.width + 64,
      height: visible.height + 64,
    };
    const part = geometry.visible(renderedView);
    const areas = mapGeometry(model.world.map).areas;
    const finishes = new Map(
      model.world.map.version === 1
        ? []
        : model.world.map.modules.map((module) => [module.area.id, module.material] as const)
    );
    // Public decking is behind room interiors. Only real openings need an
    // underlay; extending wood beneath the entire shell leaks past its alpha.
    const floors = [
      ...part.floors.filter((rect) => rect.areaId === null),
      ...part.floors.filter((rect) => rect.areaId !== null),
    ];
    if (model.world.map.version >= 6) {
      for (const source of floors) {
        if (source.areaId !== null) continue;
        const rect = geometry.floorPaintBounds(source);
        if (rect) drawBridgeBase(root, rect);
      }
    }
    for (const source of floors) {
      const rect = geometry.floorPaintBounds(source);
      if (!rect) continue;
      if (model.world.map.version >= 6 && rect.areaId === null) {
        drawBridgeDeck(root, rect, platformArt!.textures.deck);
        continue;
      }
      const finish = materials.forMaterial(finishes.get(rect.areaId ?? '') ?? 'workshop');
      const floor = createSceneFloor(finish.floor, rect);
      if (rect.areaId === null) floor.tint = '#839e98';
      root.addChild(floor);
    }
    for (const wall of part.walls.filter((wall) => wall.open)) {
      const bounds = wallProjection(wall, geometry.projection).bounds;
      if (model.world.map.version < 6) {
        const finish = materials.forMaterial(finishes.get(wall.areaId ?? '') ?? 'workshop');
        root.addChild(createSceneFloor(finish.floor, bounds));
      }
    }
    const layers: { depth: number; node: Container; frontFace?: boolean }[] = [];
    function paintLayers() {
      // A front face owns shared corner silhouettes within the architectural pass.
      for (const { node } of layers.sort(
        (a, b) => a.depth - b.depth || Number(!!a.frontFace) - Number(!!b.frontFace)
      ))
        root.addChild(node);
      layers.length = 0;
    }
    for (const wall of part.walls) {
      const node = new Container();
      if (model.world.map.version >= 6) {
        const { depth, bounds } = wallProjection(wall, geometry.projection);
        drawPlatformEdge(
          node,
          wall,
          bounds,
          platformArt!.textures,
          (light) => {
            bridgeLights.push({ light, bounds });
          },
          model.world.map.version === 8 &&
            areas.some((area) => area.id === wall.areaId && area.binding.type === 'meeting')
        );
        layers.push({ depth, node, frontFace: wall.axis === 'horizontal' });
        continue;
      }
      const finish = materials.forMaterial(
        wall.circulation ? 'workshop' : (finishes.get(wall.areaId ?? '') ?? 'workshop')
      );
      const { depth } = drawWall(
        node,
        wall,
        finish,
        geometry.projection,
        !!editor && wall.areaId === editor.areaId
      );
      layers.push({ depth, node, frontFace: wall.axis === 'horizontal' });
    }
    // Platforms support upright content; their thin rims are not occluding walls.
    // Retained cutaway maps still interleave their walls with objects and actors.
    if (model.world.map.version >= 6) paintLayers();
    // Area and actor selection sits on the platform, beneath all upright content.
    // Object handles remain a separate foreground interaction layer.
    root.addChild(groundHighlight);
    const components = new Map(model.components.map((component) => [component.id, component]));
    const functional: SceneComponent[] = [];
    for (const index of part.objects) {
      const object = model.world.objects[index]!,
        rect = geometry.objectRect(object);
      const component = components.get(object.id);
      if (component) {
        functional.push({ ...component, bounds: rect });
      }
      const group = new Container();
      const layer = new Container();
      layer.label = 'world-object';
      objectLayers.set(object.id, layer);
      layer.visible = !(pointer?.moved && pointer.rotating?.id === object.id);
      if (object.kind === 'wallLight') {
        // Nested low-opacity halos fade toward the edge without a blur filter,
        // render texture or animation loop. Keep light local to its fixture.
        const glow = new Graphics();
        for (let ring = 0; ring < 8; ring++) {
          const radius = 1 - ring / 8;
          glow
            .ellipse(
              rect.x + rect.width / 2,
              rect.y + rect.height / 2,
              rect.width * radius,
              rect.height * radius
            )
            .fill({ color: '#ffd28c', alpha: 0.014 });
        }
        layer.addChild(glow);
      }
      layer.addChild(group);
      layers.push({ depth: geometry.objectDepth(object), node: layer });
      // Every kind resolves the same immutable art and missing-art placeholder.
      // A light adds only a static glow; kind never substitutes a second asset.
      drawSceneProp(group, { ...object.placement, x: 0, y: 0 }, model.catalog, textures);
      if (object.surface.type === 'wall' && object.surface.axis === 'vertical') {
        group.rotation = object.surface.face === 'positive' ? Math.PI / 2 : -Math.PI / 2;
        group.position.set(
          rect.x + (object.surface.face === 'positive' ? rect.width : 0),
          rect.y + (object.surface.face === 'positive' ? 0 : rect.height)
        );
      } else group.position.set(rect.x, rect.y);
    }
    const actorLabels = new Container();
    const counts = new Map<string, number>();
    for (const actor of model.actors) {
      const index = counts.get(actor.areaId) ?? 0;
      counts.set(actor.areaId, index + 1);
      const slot = geometry.actorSlots(actor.areaId)[index];
      if (!slot) continue;
      const bounds = projectUpright(slot);
      if (!intersects(renderedView, bounds)) continue;
      const art = actor.avatar.customArt ?? avatarArt(actor.avatar.appearance);
      const sprite = new Sprite(textures.get(`avatar:${JSON.stringify(art)}`, art));
      sprite.position.set(bounds.x, bounds.y);
      sprite.width = bounds.width;
      sprite.height = bounds.height;
      const layer = new Container();
      layer.label = 'world-actor';
      layers.push({ depth: bounds.y + bounds.height, node: layer });
      layer.addChild(
        new Graphics()
          .ellipse(bounds.x + bounds.width / 2, bounds.y + bounds.height, 2.6, 0.6)
          .fill({ color: '#132e29', alpha: 0.3 }),
        sprite
      );
      sceneLabel(
        layer,
        actor.avatar.appearance.shirtMark,
        bounds.x + bounds.width / 2,
        bounds.y + AVATAR_LAYOUT.markCenterY,
        0.8,
        avatarMarkColor(art)
      ).anchor.set(0.5);
      nameplates.push(
        sceneActorLabel(
          actorLabels,
          `${actor.avatar.name}${actor.contractor ? ' · Contractor' : ''}`,
          actor.activity,
          bounds.x + bounds.width / 2,
          bounds.y + avatarTopInset(art) - 1
        )
      );
      actors.push({ actor, bounds });
    }
    if (editor)
      for (const slot of editor.officeSlots ?? []) {
        const visible = editor.selectedOffice ?? hoveredOffice;
        if (!visible || officeSlotKey(slot) !== officeSlotKey(visible)) continue;
        const node = new Container();
        if (model.world.map.version !== 1 && editor.selectedOffice) {
          const preview = new Graphics();
          for (const rect of officeExpansionPassages(model.world.map, slot)) {
            const passage = projectGroundRect(rect);
            preview.rect(passage.x, passage.y, passage.width, passage.height);
          }
          node.addChild(preview.fill({ color: '#5ce7ee', alpha: 0.16 }));
        }
        const bounds = drawModuleGhost(
          node,
          slot,
          Boolean(
            editor.selectedOffice && officeSlotKey(slot) === officeSlotKey(editor.selectedOffice)
          ),
          camera.scale,
          geometry.projection
        );
        layers.push({ depth: bounds.y + bounds.height, node });
      }
    if (geometry.meetingPreview && (hoveredMeeting || editor?.selectedMeeting)) {
      const meeting = geometry.meetingPreview;
      const node = new Container();
      const path = new Graphics();
      node.addChild(path);
      for (const rect of meeting.passages) {
        const projected = geometry.projection.projectGroundRect(rect, null);
        path.rect(projected.x, projected.y, projected.width, projected.height);
      }
      path.fill({ color: '#5ce7ee', alpha: 0.12 });
      const bounds = drawModuleGhost(
        node,
        meeting.slot,
        editor?.selectedMeeting?.index === meeting.slot.index,
        camera.scale,
        geometry.projection
      );
      layers.push({ depth: bounds.y + bounds.height, node });
    }
    paintLayers();
    for (const area of areas) {
      const anchor = geometry.nameplate(area.id);
      if (anchor && intersects(renderedView, { ...anchor, width: 1, height: 1 }))
        nameplates.push(
          sceneNameplate(
            root,
            area.name,
            anchor.x,
            anchor.y,
            model.world.map.version === 1 ? 24 : 36,
            'bottom-center',
            'name',
            model.world.map.version === 8 && area.binding.type === 'meeting'
          )
        );
    }
    componentLayer = drawSceneComponents(root, functional);
    componentLayer.zoom(camera.scale);
    componentLayer.highlight(hovered ?? focused);
    // Actor information is a non-interactive overlay, above architectural
    // occlusion and object affordances rather than painted into the floor.
    root.addChild(actorLabels);
    for (const plate of nameplates) plate.zoom(camera.scale);
    root.addChild(highlight, preview, catalogGhost);
    textures.end();
    selection(selected);
    refreshBridgeLights();
  }
  function applyCamera(force = false) {
    root.position.set(camera.x, camera.y);
    root.scale.set(camera.scale);
    const view = viewport();
    if (
      force ||
      !renderedView ||
      view.x < renderedView.x ||
      view.y < renderedView.y ||
      view.x + view.width > renderedView.x + renderedView.width ||
      view.y + view.height > renderedView.y + renderedView.height
    )
      draw();
    componentLayer?.zoom(camera.scale);
    for (const plate of nameplates) plate.zoom(camera.scale);
    selection(selected);
    refreshBridgeLights();
    invalidate();
  }
  function fit() {
    if (!geometry) return;
    const { projection } = geometry;
    cameraViewport = cameraFrame();
    let bounds = geometry.bounds;
    const slots = [
      ...(editor?.selectedOffice ? [editor.selectedOffice] : []),
      ...(geometry.meetingPreview ? [geometry.meetingPreview.slot] : []),
    ];
    if (slots.length) {
      const ghosts = slots.map((slot) => moduleGhostGeometry(slot, projection).bounds);
      const x = Math.min(bounds.x, ...ghosts.map((rect) => rect.x - 4));
      const y = Math.min(bounds.y, ...ghosts.map((rect) => rect.y - 4));
      const right = Math.max(
        bounds.x + bounds.width,
        ...ghosts.map((rect) => rect.x + rect.width + 4)
      );
      const bottom = Math.max(
        bounds.y + bounds.height,
        ...ghosts.map((rect) => rect.y + rect.height + 4)
      );
      bounds = { x, y, width: right - x, height: bottom - y };
    }
    camera = fitOfficeCamera(bounds, cameraViewport);
    fitted = true;
    sky();
    applyCamera(true);
  }
  function update(next: OfficeSceneModel) {
    const changed = model?.world !== next.world;
    model = next;
    if (changed) {
      geometry = worldGeometry(next.world);
      const projected = geometry;
      objectPickOrder = [...next.world.objects]
        .sort((a, b) => projected.objectDepth(a) - projected.objectDepth(b))
        .reverse();
    }
    if (!fitted) fit();
    else applyCamera(true);
  }
  function editing(next?: OfficeSceneEditor) {
    const redraw =
      Boolean(editor) !== Boolean(next) ||
      editor?.areaId !== next?.areaId ||
      editor?.officeSlots !== next?.officeSlots ||
      editor?.selectedOffice !== next?.selectedOffice ||
      editor?.selectedMeeting?.index !== next?.selectedMeeting?.index;
    editor = next;
    preview.clear();
    if (redraw) sky();
    if (redraw) applyCamera(true);
    else selection(selected);
  }
  function at(event: PointerEvent) {
    const bounds = canvas.getBoundingClientRect();
    return scenePoint(camera, event.clientX - bounds.left, event.clientY - bounds.top);
  }
  const catalogPlacement: CatalogPlacement = {
    clear() {
      for (const child of catalogGhost.removeChildren()) child.destroy({ children: true });
      catalogTextures.dispose();
      delete canvas.dataset.catalogDropValidity;
      invalidate();
    },
    preview(object, point) {
      // Pointer capture stays with the catalog card. Hit-testing the actual
      // document prevents a drop through an inspector/dialog into the canvas.
      if (
        disposed ||
        !model ||
        !geometry ||
        document.elementFromPoint(point.clientX, point.clientY) !== canvas
      ) {
        catalogPlacement.clear();
        return;
      }
      const bounds = canvas.getBoundingClientRect();
      const target = scenePoint(camera, point.clientX - bounds.left, point.clientY - bounds.top);
      const size = footprint(object.placement);
      const position = geometry.objectPosition(object, {
        x: target.x - size.width / 2,
        y: target.y - size.height / 2,
      });
      const candidate = withFurnitureBase(
        { ...object, placement: { ...object.placement, ...position } },
        model.world.map.version
      );
      const problem = placementProblem(geometry.map, candidate, model.world.objects);
      const rect = geometry.objectRect(candidate);
      const floor = geometry.projection.projectGroundRect(floorObjectBounds(candidate));
      for (const child of catalogGhost.removeChildren()) child.destroy({ children: true });
      catalogTextures.begin();
      const art = new Container();
      art.alpha = problem ? 0.45 : 0.85;
      art.position.set(rect.x, rect.y);
      drawSceneProp(art, { ...candidate.placement, x: 0, y: 0 }, model.catalog, catalogTextures);
      catalogGhost.addChild(art);
      catalogGhost.addChild(
        new Graphics()
          .rect(floor.x, floor.y, floor.width, floor.height)
          .fill({ color: problem ? '#ff525e' : '#70ddc6', alpha: 0.16 })
          .stroke({ color: problem ? '#ff8790' : '#9bffe3', width: 2 / camera.scale })
      );
      catalogTextures.end();
      canvas.dataset.catalogDropValidity = problem ? 'invalid' : 'valid';
      invalidate();
      return { object: candidate, problem };
    },
  };
  function rotationCorners(
    object = model?.world.objects.find((item) => item.id === editor?.selected)
  ) {
    const resolved = object && resolvePlacedProp(model!.catalog, object.placement);
    if (
      !object ||
      !geometry ||
      !editor?.placeObject ||
      !resolved ||
      !canRotateObject(object, model!.catalog) ||
      object.surface.type !== 'floor'
    )
      return [];
    const rect = geometry.objectRect(object);
    return [
      { x: rect.x, y: rect.y },
      { x: rect.x + rect.width, y: rect.y },
      { x: rect.x + rect.width, y: rect.y + rect.height },
      { x: rect.x, y: rect.y + rect.height },
    ];
  }
  function rotationAt(point: { x: number; y: number }) {
    return rotationCorners().some(
      (corner) => Math.hypot(point.x - corner.x, point.y - corner.y) <= 9 / camera.scale
    );
  }
  let hoveredOffice: OfficeSlot | undefined;
  let hoveredMeeting = false;
  let pointer:
    | {
        id: number;
        x: number;
        y: number;
        originX: number;
        originY: number;
        start: { x: number; y: number };
        objectId?: string;
        selecting: boolean;
        moved: boolean;
        rotating?: WorldObject;
        rotationPreview?: WorldObject;
      }
    | undefined;
  function objectAt(point: { x: number; y: number }) {
    if (!geometry) return;
    const projected = geometry;
    return objectPickOrder.find((item) =>
      intersects(projected.objectRect(item), { ...point, width: 0.001, height: 0.001 })
    );
  }
  function actorAt(point: { x: number; y: number }) {
    return [...actors]
      .reverse()
      .find((entry) => intersects(entry.bounds, { ...point, width: 0.001, height: 0.001 }));
  }
  function officeAt(point: { x: number; y: number }) {
    if (!geometry) return;
    const projected = geometry;
    return editor?.officeSlots?.find((slot) =>
      intersects(moduleGhostGeometry(slot, projected.projection).bounds, {
        ...point,
        width: 0.001,
        height: 0.001,
      })
    );
  }
  function meetingAt(point: { x: number; y: number }) {
    return geometry?.meetingPreview &&
      intersects(moduleGhostGeometry(geometry.meetingPreview.slot, geometry.projection).bounds, {
        ...point,
        width: 0.001,
        height: 0.001,
      })
      ? geometry.meetingPreview.slot
      : undefined;
  }
  function movedObject(point: { x: number; y: number }) {
    const object = model?.world.objects.find((item) => item.id === pointer?.objectId);
    if (!object || !pointer || !geometry) return;
    const rect = geometry.objectRect(object);
    const position = geometry.objectPosition(object, {
      x: rect.x + point.x - pointer.start.x,
      y: rect.y + point.y - pointer.start.y,
    });
    const candidate = withFurnitureBase(
      { ...object, placement: { ...object.placement, ...position } },
      model!.world.map.version
    );
    return {
      object: candidate,
      problem: placementProblem(geometry.map, candidate, model!.world.objects),
      rect: geometry.objectRect(candidate),
    };
  }
  function rotatedAt(point: { x: number; y: number }) {
    const original = pointer?.rotating;
    if (!original || !geometry || !pointer) return;
    const rect = geometry.objectRect(original);
    const center = { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
    const angle =
      Math.atan2(point.y - center.y, point.x - center.x) -
      Math.atan2(pointer.start.y - center.y, pointer.start.x - center.x);
    const object = rotatedObject(
      original,
      Math.round(angle / (Math.PI / 2)),
      model!.catalog,
      model!.world.map.version
    );
    return { object, problem: placementProblem(geometry.map, object, model!.world.objects) };
  }
  const down = (event: PointerEvent) => {
    if (pointer || (event.button !== 0 && event.button !== 1)) return;
    const point = at(event);
    pointer = {
      id: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      originX: camera.x,
      originY: camera.y,
      start: point,
      moved: false,
      rotating:
        event.button === 0 && !event.shiftKey && rotationAt(point)
          ? model?.world.objects.find((item) => item.id === editor?.selected)
          : undefined,
      selecting: event.button === 0 && !event.shiftKey,
      objectId:
        editor && event.button === 0 && !event.shiftKey && !actorAt(point)
          ? objectAt(point)?.id
          : undefined,
    };
    canvas.focus({ preventScroll: true });
    canvas.setPointerCapture(event.pointerId);
  };
  const move = (event: PointerEvent) => {
    const point = at(event);
    if (!pointer) {
      hovered = componentLayer?.pick(point.x, point.y);
      const office = officeAt(point);
      const meeting = Boolean(meetingAt(point));
      const changed = office !== hoveredOffice || meeting !== hoveredMeeting;
      hoveredOffice = office;
      hoveredMeeting = meeting;
      canvas.style.cursor = rotationAt(point)
        ? 'crosshair'
        : objectAt(point)
          ? 'grab'
          : office || meeting || hovered || actorAt(point)
            ? 'pointer'
            : 'grab';
      if (changed) draw();
      interaction(focused);
      return;
    }
    if (pointer.id !== event.pointerId) return;
    const x = event.clientX - pointer.x,
      y = event.clientY - pointer.y;
    if (Math.hypot(x, y) > 5) pointer.moved = true;
    if (!pointer.moved) return;
    if (pointer.rotating) {
      const candidate = rotatedAt(point);
      if (!candidate || !geometry) return;
      const { object, problem } = candidate;
      pointer.rotationPreview = object;
      selection(selected);
      const rect = geometry.objectRect(object);
      const originalLayer = objectLayers.get(pointer.rotating.id);
      if (originalLayer) originalLayer.visible = false;
      for (const child of catalogGhost.removeChildren()) child.destroy({ children: true });
      catalogTextures.begin();
      const art = new Container();
      art.position.set(rect.x, rect.y);
      drawSceneProp(art, { ...object.placement, x: 0, y: 0 }, model!.catalog, catalogTextures);
      catalogGhost.addChild(art);
      catalogTextures.end();
      preview
        .clear()
        .rect(rect.x, rect.y, rect.width, rect.height)
        .stroke({ color: problem ? '#ff525e' : '#9bffe3', width: 2 / camera.scale });
      canvas.dataset.dropValidity = problem ? 'invalid' : 'valid';
      canvas.style.cursor = problem ? 'not-allowed' : 'crosshair';
      invalidate();
      return;
    }
    if (pointer.objectId) {
      const moved = movedObject(point);
      if (!moved) return;
      const { rect, problem } = moved;
      canvas.dataset.dropValidity = problem ? 'invalid' : 'valid';
      canvas.style.cursor = problem ? 'not-allowed' : 'grabbing';
      preview
        .clear()
        .rect(rect.x, rect.y, rect.width, rect.height)
        .fill({ color: problem ? '#ff525e' : '#70ddc6', alpha: 0.3 })
        .stroke({ color: problem ? '#ff8790' : '#c9fff1', width: 0.25 });
      invalidate();
    } else {
      camera = { ...camera, x: pointer.originX + x, y: pointer.originY + y };
      applyCamera();
    }
  };
  const up = (event: PointerEvent) => {
    if (!pointer || pointer.id !== event.pointerId) return;
    restoreRotationSource();
    const point = at(event);
    if (event.type !== 'pointercancel') {
      if (pointer.rotating) {
        const candidate = pointer.moved && rotatedAt(point);
        if (
          candidate &&
          !candidate.problem &&
          candidate.object.placement.rotation !== pointer.rotating.placement.rotation
        )
          editor?.placeObject(candidate.object);
      } else if (pointer.moved && pointer.objectId) {
        const moved = movedObject(point);
        if (moved && !moved.problem) {
          editor?.select?.(moved.object.id);
          editor?.placeObject(moved.object);
        }
      } else if (!pointer.moved && pointer.selecting) {
        const actor = actorAt(point);
        const object = objectAt(point);
        const meeting = meetingAt(point);
        const office = officeAt(point);
        if (actor)
          select({ kind: 'agent', identityId: actor.actor.identityId, areaId: actor.actor.areaId });
        else if (object && editor) editor.select?.(object.id);
        else if (meeting) createMeeting(meeting);
        else if (office) editor?.chooseOffice?.(office);
        else {
          const component = componentLayer?.pick(point.x, point.y);
          const ground = unprojectGround(point);
          const area = geometry?.map.areaAt(Math.floor(ground.x), Math.floor(ground.y));
          if (component) activate(component);
          else if (area) select({ kind: 'area', areaId: area });
          else editor?.clearSelection?.();
        }
      }
    }
    preview.clear();
    catalogPlacement.clear();
    delete canvas.dataset.dropValidity;
    canvas.style.cursor = 'grab';
    invalidate();
    if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
    pointer = undefined;
    selection(selected);
  };
  const leave = () => {
    if (pointer) return;
    if (hoveredOffice || hoveredMeeting) {
      hoveredOffice = undefined;
      hoveredMeeting = false;
      draw();
    }
  };
  const lostGesture = () => {
    restoreRotationSource();
    const current = pointer;
    pointer = undefined;
    if (current && canvas.hasPointerCapture(current.id)) canvas.releasePointerCapture(current.id);
    preview.clear();
    catalogPlacement.clear();
    delete canvas.dataset.dropValidity;
    selection(selected);
    invalidate();
  };
  const cancelGesture = (event: KeyboardEvent) => {
    if (event.key !== 'Escape') return;
    restoreRotationSource();
    if (pointer && canvas.hasPointerCapture(pointer.id)) canvas.releasePointerCapture(pointer.id);
    pointer = undefined;
    preview.clear();
    catalogPlacement.clear();
    delete canvas.dataset.dropValidity;
    canvas.style.cursor = 'grab';
    hoveredOffice = undefined;
    hoveredMeeting = false;
    editor?.clearSelection?.();
    draw();
    event.stopPropagation();
  };
  function restoreRotationSource() {
    const layer = pointer?.rotating && objectLayers.get(pointer.rotating.id);
    if (layer) layer.visible = true;
  }
  function zoom(factor: number, x = host.clientWidth / 2, y = host.clientHeight / 2) {
    if (!geometry) return;
    const fitted = fitOfficeCamera(geometry.bounds, cameraFrame());
    camera = zoomOfficeCamera(camera, factor, x, y, fitted.scale, Math.max(fitted.scale, 8));
    applyCamera(true);
  }
  const wheel = (event: WheelEvent) => {
    event.preventDefault();
    if (!geometry) return;
    const bounds = canvas.getBoundingClientRect();
    const fitted = fitOfficeCamera(geometry.bounds, cameraFrame());
    camera = wheelOfficeCamera(
      camera,
      event,
      event.clientX - bounds.left,
      event.clientY - bounds.top,
      bounds.height,
      fitted.scale,
      Math.max(fitted.scale, 8)
    );
    applyCamera(true);
  };
  const resize = () => {
    const next = cameraFrame();
    if (geometry) camera = resizeOfficeCamera(camera, geometry.bounds, cameraViewport, next);
    cameraViewport = next;
    sky();
    applyCamera(true);
  };
  canvas.addEventListener('pointerdown', down);
  canvas.addEventListener('pointermove', move);
  canvas.addEventListener('pointerleave', leave);
  canvas.addEventListener('keydown', cancelGesture);
  canvas.addEventListener('pointerup', up);
  canvas.addEventListener('pointercancel', up);
  canvas.addEventListener('lostpointercapture', lostGesture);
  window.addEventListener('blur', lostGesture);
  canvas.addEventListener('wheel', wheel, { passive: false });
  application.renderer.on('resize', resize);
  function dispose() {
    if (disposed) return;
    disposed = true;
    signal.removeEventListener('abort', dispose);
    canvas.removeEventListener('pointerdown', down);
    canvas.removeEventListener('pointermove', move);
    canvas.removeEventListener('pointerleave', leave);
    canvas.removeEventListener('keydown', cancelGesture);
    canvas.removeEventListener('pointerup', up);
    canvas.removeEventListener('pointercancel', up);
    canvas.removeEventListener('lostpointercapture', lostGesture);
    window.removeEventListener('blur', lostGesture);
    canvas.removeEventListener('wheel', wheel);
    if (application.renderer) application.renderer.off('resize', resize);
    bridgePulse.dispose();
    catalogTextures.dispose();
    objectLayers.clear();
    disposeApplication();
    textures.dispose();
    materials?.dispose();
    platformArt?.dispose();
  }
  signal.addEventListener('abort', dispose, { once: true });
  return {
    update,
    selection,
    interaction,
    editing,
    fit,
    zoom,
    catalogPlacement,
    dispose,
  };
}
