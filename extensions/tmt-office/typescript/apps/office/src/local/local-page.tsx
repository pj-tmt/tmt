import { useEffect, useMemo, useRef, useState } from 'react';
import { WorldObjectActions } from '../extensions/world-object-actions.js';
import type { ProfileSnapshot } from '../profiles/profile-contract.js';
import type { IdentityChoice } from '../profiles/identity-choice.js';
import { useLocalOffice } from './use-local-office.js';
import { OfficeCanvas } from './office-canvas.js';
import { WorldHud } from './world-hud.js';
import { mapGeometry } from '../world-map/map-source.js';
import { officeSceneModel } from './office-scene-model.js';
import { officePopulation } from './office-population.js';
import { AreaRoster, OfficeDirectory } from './office-directory.js';
import type { OfficeSelection } from '../rendering/office-selection.js';
import { useAgentConversation } from './use-agent-conversation.js';
import type { ConversationTarget } from './agent-conversation.js';
import { AgentInfo, AgentPortrait } from './agent-info.js';
import { useStatusClock } from '../identities/use-status-clock.js';
import { useExtensionPanel } from '../extensions/use-extension-panel.js';
import { RoomPicker } from './room-picker.js';
import { useRoomMessage } from './room-message.js';
import type { MeetingRoom } from './room-contract.js';
import { useWorldExtensions } from './use-world-extensions.js';
import { useWorldEditor } from '../world-map/use-world-editor.js';
import { WorldTools } from '../world-map/world-tools.js';
import { OfficeExpansionForm } from '../world-map/office-expansion-form.js';
import { AreaCreationForm } from '../world-map/area-creation-form.js';
import { MeetingCreationForm } from '../world-map/meeting-creation-form.js';
import { addMeetingModule, nextMeetingSlot } from '../world-map/meeting-module.js';
import {
  addOfficeModule,
  addGridMeetingModule,
  officeExpansionSlots,
} from '../world-map/module-authoring.js';
import { officeSlotKey } from '../world-map/module-contract.js';
import type { MeetingSlot, OfficeSlot } from '../world-map/module-contract.js';
import { createCatalogObject, placementProblem } from '../world-map/world-object-placement.js';
import { projectMap } from '../world-map/map-geometry.js';
import { useCatalogDrag } from '../world-map/use-catalog-drag.js';
import { removeWorldObject } from '../world-map/world-draft.js';
import { objectArea } from '../world-map/object-area.js';
import { indexFloor } from '../world-map/floor-index.js';
import { PixelWorkshop } from '../props/pixel-workshop.js';
import type { CatalogPack } from '../props/prop-contract.js';
import type { OfficeSceneEditor } from '../rendering/office-scene.js';
import '../blocks/block.css';
import '../profiles/profile.css';
import './office-floor.css';
import '../world-map/world-tools.css';

type Ready = Extract<ReturnType<typeof useLocalOffice>['load'], { status: 'ready' }>;

export function LocalOfficePage({ initialIdentityId }: { initialIdentityId?: string }) {
  const { load, refresh, profileChanged, roomChanged, propObserved } = useLocalOffice();
  if (load.status === 'loading') return <p role="status">Opening your office…</p>;
  if (load.status === 'error')
    return (
      <section>
        <h1>Your office</h1>
        <p role="alert">Local Office could not load. Check the service or try again.</p>
        <button onClick={refresh}>Try again</button>
      </section>
    );
  return (
    <ReadyOffice
      load={load}
      refresh={refresh}
      profileChanged={profileChanged}
      roomChanged={roomChanged}
      propObserved={propObserved}
      initialIdentityId={initialIdentityId}
    />
  );
}

function ReadyOffice({
  load,
  refresh,
  profileChanged,
  roomChanged,
  propObserved,
  initialIdentityId,
}: {
  load: Ready;
  refresh: () => void;
  profileChanged: (profile: ProfileSnapshot) => void;
  roomChanged: (room: MeetingRoom) => void;
  propObserved: (pack: CatalogPack) => void;
  initialIdentityId?: string;
}) {
  const worldEditor = useWorldEditor(load.world, load.runtime.world);
  const [roomBusy, setRoomBusy] = useState(false);
  const editor = {
    ...worldEditor,
    busy: worldEditor.busy || roomBusy,
  };
  const { undo, redo, busy } = editor;
  useEffect(() => {
    const historyKey = (event: KeyboardEvent) => {
      const target = event.target;
      if (
        event.defaultPrevented ||
        !(event.metaKey || event.ctrlKey) ||
        event.altKey ||
        event.key.toLowerCase() !== 'z' ||
        !(target instanceof HTMLElement) ||
        target.closest('input, textarea, select, [contenteditable], dialog, [role="dialog"]')
      )
        return;
      event.preventDefault();
      if (!busy) (event.shiftKey ? redo : undo)();
    };
    // Disabling the last Undo button can return focus to body. History shortcuts
    // must still work there, without taking over text or resource-editor history.
    window.addEventListener('keydown', historyKey);
    return () => window.removeEventListener('keydown', historyKey);
  }, [undo, redo, busy]);
  const [cameraHost, setCameraHost] = useState<HTMLDivElement | null>(null);
  const above = useRef<HTMLDivElement>(null);
  const editorViewport = useRef<HTMLDivElement>(null);
  const panelObstacles = useMemo(() => ({ above, viewport: editorViewport }), []);
  const { world } = editor;
  const map = mapGeometry(world.map);
  const extensions = useWorldExtensions(world, load.props);
  const roomMessage = useRoomMessage();
  const [managedRoom, setManagedRoom] = useState<{ roomId?: string; addMember?: IdentityChoice }>(
    {}
  );
  const [roomDraftDirty, setRoomDraftDirty] = useState(false);
  const [roomNotice, setRoomNotice] = useState<string>();
  const meetingRooms = useExtensionPanel(
    'Meeting rooms',
    <>
      <h2>Meeting rooms</h2>
      {roomNotice && <p role="status">{roomNotice}</p>}
      <p>Save room updates membership, not your office layout. Closing keeps your draft.</p>
      <RoomPicker
        key={`${managedRoom.roomId ?? 'all'}:${managedRoom.addMember?.id ?? ''}`}
        roomId={managedRoom.roomId}
        addMember={managedRoom.addMember}
        onDraftChange={setRoomDraftDirty}
        purpose="manage"
        port={load.runtime.rooms}
        choices={load.profiles.map((profile) => ({
          id: profile.identityId,
          name: profile.identityName,
          presence: profile.presence,
        }))}
        onSaved={roomChanged}
      />
    </>,
    'meeting-manager'
  );
  function openMeetingRoom(roomId?: string, addMember?: IdentityChoice) {
    if (
      roomDraftDirty &&
      (roomId !== managedRoom.roomId || addMember?.id !== managedRoom.addMember?.id)
    )
      setRoomNotice('Finish or discard the current room draft before switching rooms.');
    else {
      setManagedRoom({ roomId, addMember });
      setRoomNotice(undefined);
    }
    meetingRooms.open();
  }
  const [selection, setSelection] = useState<OfficeSelection | undefined>(
    initialIdentityId ? { kind: 'agent', identityId: initialIdentityId } : undefined
  );
  const directoryToggle = useRef<HTMLButtonElement>(null);
  const [officeMenuOpen, setOfficeMenuOpen] = useState(false);
  const [panel, setPanel] = useState<'directory' | 'details' | null>(null);
  const [officeSlot, setOfficeSlot] = useState<OfficeSlot>();
  const [meetingDraft, setMeetingDraft] = useState<MeetingSlot>();
  const meetingSlot = useMemo(
    () =>
      world.map.version !== 1 && world.map.version >= 4 ? nextMeetingSlot(world.map) : undefined,
    [world.map]
  );
  function beginMeeting(slot: MeetingSlot) {
    if (editor.busy || slot.index !== meetingSlot?.index) return;
    setPanel(null);
    setOfficeSlot(undefined);
    setMeetingDraft(slot);
  }
  const expanding = Boolean(officeSlot) && world.map.version !== 1;
  const officeSlots = useMemo(
    () => (world.map.version !== 1 ? officeExpansionSlots(world.map) : []),
    [world.map]
  );
  const selectedOffice = expanding
    ? officeSlots.find((slot) => officeSlot && officeSlotKey(slot) === officeSlotKey(officeSlot))
    : undefined;
  const [chosenAreaId, setChosenAreaId] = useState<string>(world.map.primaryLobbyId);
  const [objectId, setObjectId] = useState<string>();
  function selectObject(id?: string) {
    setObjectId(id);
    setSelection(undefined);
    setPanel(null);
    setOfficeSlot(undefined);
    setMeetingDraft(undefined);
  }
  const floor = useMemo(() => indexFloor(map.floor), [map.floor]);
  const selectedObject = world.objects.find((object) => object.id === objectId);
  const objectAreaId = selectedObject ? objectArea(floor, selectedObject) : undefined;
  const areaId =
    objectAreaId === undefined
      ? map.areas.some((area) => area.id === chosenAreaId)
        ? chosenAreaId
        : world.map.primaryLobbyId
      : (objectAreaId ?? '');
  function selectArea(id: string) {
    setChosenAreaId(id);
    setObjectId(undefined);
    setSelection({ kind: 'area', areaId: id });
    setPanel('details');
    setOfficeSlot(undefined);
    setMeetingDraft(undefined);
  }
  function addArt(pack: CatalogPack, key: string) {
    if (editor.busy) throw new Error('Wait for the current room operation to finish.');
    const id = crypto.randomUUID();
    const object = createCatalogObject(world, pack, key, areaId || null, id);
    if (!editor.change((current) => ({ ...current, objects: [...current.objects, object] })))
      throw new Error('This artwork could not be added. Check the layout validation message.');
    propObserved(pack);
    selectObject(id);
  }
  const catalogDrag = useCatalogDrag({
    disabled: editor.busy,
    commit: (object, pack) => {
      const changed = editor.change((current) => {
        const problem = placementProblem(projectMap(current.map), object, current.objects);
        if (problem) throw new Error(problem);
        return { ...current, objects: [...current.objects, object] };
      });
      if (!changed)
        throw new Error('This object could not be added. Check the layout validation message.');
      propObserved(pack);
      selectObject(object.id);
    },
  });
  const workshop = useExtensionPanel(
    'Pixel workshop',
    <PixelWorkshop port={load.runtime.propCatalog} canAdd={!editor.busy} add={addArt} />
  );
  const population = useMemo(
    () => officePopulation(world.map, load.profiles, load.rooms),
    [world.map, load.profiles, load.rooms]
  );
  const statusNowMs = useStatusClock(load.profiles);
  const selectedArea =
    selection?.kind === 'area' ? population.areas.get(selection.areaId) : undefined;
  const conversation = useAgentConversation(load.runtime, {
    refreshInfo: refresh,
    refreshingInfo: load.refreshing || editor.busy || editor.dirty || editor.saving,
    initial: initialIdentityId
      ? targetFor({ kind: 'agent', identityId: initialIdentityId })
      : undefined,
    suspended: selection?.kind !== 'agent' || panel !== null || Boolean(meetingDraft),
    closed: () => {
      setSelection(undefined);
      setPanel(null);
      directoryToggle.current?.focus();
    },
    portrait: (target) => {
      const profile = population.identities.get(target.id);
      return profile ? <AgentPortrait profile={profile} catalog={load.avatars} /> : null;
    },
    info: (target) => {
      const profile = population.identities.get(target.id);
      if (!profile)
        return <p>This identity is unavailable. Retained exchanges are still readable.</p>;
      const area = population.areas.get(target.areaId ?? population.homes.get(target.id)!);
      return (
        <AgentInfo
          key={target.id}
          profile={profile}
          port={load.runtime.profiles}
          catalog={load.avatars}
          changed={profileChanged}
          nowMs={statusNowMs}
        >
          {area?.room && (
            <p>Viewing in {area.room.name}. Membership is independent of the home area.</p>
          )}
          <button
            onClick={() =>
              openMeetingRoom(undefined, { id: profile.identityId, name: profile.identityName })
            }
          >
            Add to meeting…
          </button>
          {area && (
            <button onClick={() => select({ kind: 'area', areaId: area.area.id })}>
              View {area.area.name} roster
            </button>
          )}
          <WorldObjectActions
            groups={extensions.groups}
            areaId={area?.area.id}
            activate={extensions.activate}
            focus={extensions.focus}
          />
        </AgentInfo>
      );
    },
  });
  const conversationIdentityId = conversation.identityId;
  const sceneModel = useMemo(
    () =>
      officeSceneModel(
        population,
        world,
        load.avatars,
        load.props,
        extensions.components,
        statusNowMs,
        conversationIdentityId
      ),
    [
      population,
      world,
      load.avatars,
      load.props,
      extensions.components,
      statusNowMs,
      conversationIdentityId,
    ]
  );
  function targetFor(
    value: Extract<OfficeSelection, { kind: 'agent' }>
  ): ConversationTarget | undefined {
    const identity = population.identities.get(value.identityId);
    if (!identity) return;
    const area = population.areas.get(value.areaId ?? population.homes.get(value.identityId)!);
    return {
      id: identity.identityId,
      name: identity.identityName,
      areaId: area?.area.id,
      ...(area?.room ? { room: { id: area.room.id, name: area.room.name } } : {}),
    };
  }
  function select(value: OfficeSelection, tab: 'chat' | 'info' = 'info') {
    setSelection(value);
    if (value.kind === 'area') {
      setPanel('details');
      selectArea(value.areaId);
    } else {
      const target = targetFor(value);
      if (!target) return;
      setObjectId(undefined);
      setOfficeSlot(undefined);
      setMeetingDraft(undefined);
      setPanel(null);
      conversation.open(target, tab);
    }
  }
  function closePanel() {
    setSelection(undefined);
    setObjectId(undefined);
    setOfficeSlot(undefined);
    setMeetingDraft(undefined);
    setPanel(null);
    directoryToggle.current?.focus();
  }
  const sceneEditor: OfficeSceneEditor = {
    areaId,
    selectedMeeting: meetingDraft,
    officeSlots,
    selectedOffice,
    chooseOffice: (slot) => {
      if (!editor.busy) {
        closePanel();
        setOfficeSlot(slot);
      }
    },
    clearSelection: () => {
      if (!editor.busy) closePanel();
    },
    selected: selectedObject?.id,
    placeObject: (candidate) =>
      !editor.busy &&
      editor.change((world) => ({
        ...world,
        objects: world.objects.map((object) => (object.id === candidate.id ? candidate : object)),
      })),
    select: (id) => {
      if (!editor.busy) selectObject(id);
    },
  };
  return (
    <section
      className="office-overview"
      aria-label="Office overview"
      onKeyDown={(event) => {
        const target = event.target as HTMLElement;
        if (
          (event.key === 'Delete' || event.key === 'Backspace') &&
          selectedObject &&
          !editor.busy &&
          !event.defaultPrevented &&
          !event.nativeEvent.isComposing &&
          !target.closest('input, textarea, select, [contenteditable], dialog, [role="dialog"]')
        ) {
          event.preventDefault();
          if (editor.change((world) => removeWorldObject(world, selectedObject.id)))
            selectObject(undefined);
          return;
        }
        if (event.key === 'Escape' && !roomBusy) {
          event.stopPropagation();
          closePanel();
        }
      }}
    >
      <h1 className="world-heading">Your office</h1>
      <div className="world-hud-layout">
        <div className="world-hud-topline">
          <WorldHud>
            <div
              className="world-navigation"
              onKeyDown={(event) => {
                if (event.key === 'Escape' && officeMenuOpen) {
                  event.stopPropagation();
                  setOfficeMenuOpen(false);
                  directoryToggle.current?.focus();
                }
              }}
            >
              <button
                ref={directoryToggle}
                aria-expanded={officeMenuOpen}
                aria-controls="office-navigation"
                disabled={editor.busy}
                onClick={() => setOfficeMenuOpen(!officeMenuOpen)}
              >
                Office menu
              </button>
              <div
                id="office-navigation"
                className="world-navigation-options"
                hidden={!officeMenuOpen}
              >
                <button
                  aria-expanded={panel === 'directory'}
                  aria-controls="office-directory"
                  onClick={() => {
                    setPanel(panel === 'directory' ? null : 'directory');
                    setOfficeMenuOpen(false);
                    directoryToggle.current?.focus();
                  }}
                >
                  Directory · {load.profiles.length}
                </button>
                <button
                  disabled={editor.busy}
                  onClick={() => {
                    setOfficeMenuOpen(false);
                    directoryToggle.current?.focus();
                    openMeetingRoom();
                  }}
                >
                  Meeting rooms
                </button>
              </div>
            </div>
            <span className="world-population">
              {map.areas.length} areas ·{' '}
              {load.profiles.filter((profile) => profile.presence === 'active').length} online
            </span>
            <button
              aria-label="Refresh office"
              disabled={editor.dirty || editor.saving || editor.busy || load.refreshing}
              onClick={refresh}
            >
              ↻
            </button>
            {load.refreshing && <span role="status">Refreshing…</span>}
            {load.refreshError && <span role="alert">{load.refreshError}</span>}
          </WorldHud>
          <div className="world-camera-dock" ref={setCameraHost} />
        </div>
        <WorldTools
          inspector={conversation.render()}
          inspecting={conversation.visible}
          roomPort={load.runtime.rooms}
          roomSaved={roomChanged}
          onRoomBusyChange={setRoomBusy}
          obstacles={panelObstacles}
          creatingMeeting={Boolean(meetingDraft)}
          editor={editor}
          clearSelection={closePanel}
          content={
            selectedObject?.extension ? (
              <WorldObjectActions
                groups={extensions.groups
                  .map((group) => ({
                    ...group,
                    entries: group.entries.filter((entry) => entry.instance.id === objectId),
                  }))
                  .filter((group) => group.entries.length)}
                areaId={areaId}
                activate={extensions.activate}
                focus={extensions.focus}
              />
            ) : selectedArea ? (
              <>
                <AreaRoster population={selectedArea} select={select} />
                {selectedArea.room && (
                  <>
                    <button onClick={() => openMeetingRoom(selectedArea.room!.id)}>
                      Manage members
                    </button>
                    <button onClick={() => roomMessage.open(selectedArea.room!)}>
                      Message room
                    </button>
                  </>
                )}
                <WorldObjectActions
                  groups={extensions.groups}
                  areaId={selectedArea.area.id}
                  activate={extensions.activate}
                  focus={extensions.focus}
                />
              </>
            ) : undefined
          }
          areaId={selectedObject ? areaId : selectedArea?.area.id}
          selectArea={selectArea}
          objectId={selectedObject?.id}
          selectObject={selectObject}
          population={population}
          rooms={load.rooms}
          catalog={load.props}
          addArt={addArt}
          catalogDrag={catalogDrag.handlers}
          openWorkshop={workshop.open}
        />
        <aside
          className="world-directory"
          id="office-directory"
          hidden={panel !== 'directory'}
          aria-label="Office directory"
        >
          <OfficeDirectory population={population} selection={selection} select={select} />
          <details>
            <summary>Available spaces</summary>
            {officeSlots.map((slot) => (
              <button
                key={officeSlotKey(slot)}
                onClick={() => {
                  closePanel();
                  setOfficeSlot(slot);
                }}
              >
                {world.map.version === 8 ? 'Area' : 'Office'} · column {slot.column}, row {slot.row}
              </button>
            ))}
            {meetingSlot && (
              <button onClick={() => beginMeeting(meetingSlot)}>New meeting room</button>
            )}
          </details>
        </aside>
      </div>
      <OfficeCanvas
        placementRef={catalogDrag.placement}
        cameraHost={cameraHost}
        model={sceneModel}
        select={(value) => {
          if (!editor.busy) select(value, 'chat');
        }}
        selection={selection}
        editor={sceneEditor}
        activate={extensions.activate}
        focusedComponentId={extensions.focused}
        createMeeting={beginMeeting}
        meetingOverlay={(anchor) =>
          meetingDraft ? (
            <MeetingCreationForm
              key={meetingDraft.index}
              port={load.runtime.rooms}
              rooms={load.rooms.filter(
                (room) =>
                  !map.areas.some(
                    (area) => area.binding.type === 'meeting' && area.binding.roomId === room.id
                  )
              )}
              anchor={anchor}
              obstacles={panelObstacles}
              busy={editor.busy}
              onBusyChange={setRoomBusy}
              roomSaved={roomChanged}
              close={() => setMeetingDraft(undefined)}
              place={(room, id) => {
                if (!editor.change((current) => addMeetingModule(current, meetingDraft, room, id)))
                  return false;
                selectArea(id);
                setMeetingDraft(undefined);
                return true;
              }}
            />
          ) : null
        }
        officeOverlay={
          selectedOffice
            ? (anchor) =>
                world.map.version === 8 ? (
                  <AreaCreationForm
                    key={officeSlotKey(selectedOffice)}
                    selected={selectedOffice}
                    anchor={anchor}
                    obstacles={panelObstacles}
                    busy={editor.busy || roomBusy}
                    port={load.runtime.rooms}
                    onBusyChange={setRoomBusy}
                    roomSaved={roomChanged}
                    rooms={load.rooms.filter(
                      (room) =>
                        !map.areas.some(
                          (area) =>
                            area.binding.type === 'meeting' && area.binding.roomId === room.id
                        )
                    )}
                    close={() => setOfficeSlot(undefined)}
                    createOffice={(name) => {
                      const id = crypto.randomUUID();
                      if (
                        editor.change((current) =>
                          addOfficeModule(current, selectedOffice, name, id)
                        )
                      ) {
                        selectArea(id);
                        setOfficeSlot(undefined);
                      }
                    }}
                    createMeeting={(room, id) => {
                      if (
                        !editor.change((current) =>
                          addGridMeetingModule(current, selectedOffice, room, id)
                        )
                      )
                        return false;
                      selectArea(id);
                      setOfficeSlot(undefined);
                      return true;
                    }}
                  />
                ) : (
                  <OfficeExpansionForm
                    key={selectedOffice ? officeSlotKey(selectedOffice) : 'choose'}
                    selected={selectedOffice}
                    anchor={anchor}
                    obstacles={panelObstacles}
                    busy={editor.busy}
                    cancel={() => {
                      setOfficeSlot(undefined);
                    }}
                    create={(name) => {
                      if (!selectedOffice) return;
                      const id = crypto.randomUUID();
                      if (
                        editor.change((world) => addOfficeModule(world, selectedOffice, name, id))
                      ) {
                        selectArea(id);
                        setOfficeSlot(undefined);
                      }
                    }}
                  />
                )
            : undefined
        }
      />
      {catalogDrag.feedback && (
        <div
          className="catalog-drag-feedback"
          role="status"
          data-valid={catalogDrag.feedback.valid}
          style={{
            left: Math.max(8, Math.min(catalogDrag.feedback.x + 18, window.innerWidth - 260)),
            top: Math.max(8, Math.min(catalogDrag.feedback.y + 24, window.innerHeight - 64)),
          }}
        >
          {catalogDrag.feedback.valid ? '✓ ' : '× '}
          {catalogDrag.feedback.message}
        </div>
      )}
      {catalogDrag.error && (
        <p className="scene-status" role="alert">
          {catalogDrag.error}
        </p>
      )}
      {extensions.panel}
      {workshop.panel}
      {meetingRooms.panel}
      {roomMessage.panel}
    </section>
  );
}
