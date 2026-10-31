import { ClientError, RefusalError, ReopenSessionError, budget, invitationPage, invitations, management, members, reopenSession } from "/sdk/remote-v1.js";
//#region src/remote-routes.ts
var REMOTE_ITEMS = [
	{
		id: "remotes",
		path: "/remote/",
		label: "My Remotes",
		title: "My Remotes",
		page: "settings"
	},
	{
		id: "create",
		path: "/remote/create",
		label: "Create a Remote",
		title: "Create a Remote",
		page: "settings"
	},
	{
		id: "join",
		path: "/remote/join",
		label: "Join a Remote",
		title: "Join a Remote",
		page: "settings"
	},
	{
		id: "overview",
		path: "/remote/overview",
		label: "Overview",
		title: "Overview",
		page: "landing"
	},
	{
		id: "members",
		path: "/remote/members",
		label: "People",
		title: "People",
		page: "settings"
	},
	{
		id: "invites",
		path: "/remote/invites",
		label: "Invites",
		title: "Invites",
		page: "settings",
		adminOnly: true
	},
	{
		id: "my-devices",
		path: "/remote/my-devices",
		label: "My devices",
		title: "My devices",
		page: "settings"
	},
	{
		id: "service",
		path: "/remote/service",
		label: "Service & upgrade",
		title: "Service & upgrade",
		page: "settings",
		adminOnly: true
	},
	{
		id: "settings",
		path: "/remote/settings",
		label: "Settings",
		title: "Settings",
		page: "settings"
	}
];
[...REMOTE_ITEMS.filter((item) => item.id !== "remotes").map((item) => item.path), remotePath("remotes")];
function remotePath(id) {
	return REMOTE_ITEMS.find((item) => item.id === id).path;
}
/** Removed pre-launch paths have no alias or redirect. */
function remoteItem(pathname) {
	return REMOTE_ITEMS.find((item) => item.path === pathname || pathname === "/remote" && item.id === "remotes");
}
//#endregion
//#region src/remote-state.ts
/** A display observation only; pairing and sending permissions never imply a role. */
function remoteRole(role) {
	return role === "owner" ? "Admin" : role === "user" ? "Member" : "Role unknown";
}
/** M1 browser-role display from the existing readable management observation; grants nothing. */
function managementRole(access, settingsWrite) {
	if (access !== "live") return void 0;
	return settingsWrite === true ? "owner" : settingsWrite === false ? "user" : void 0;
}
/** Foreign references stay unchecked. Unknown input does not become an offline verdict. */
function remoteState(state, current = true) {
	if (!current) return {
		text: "Not checked",
		word: "not checked",
		mark: "◌",
		tone: "muted",
		action: ""
	};
	const known = {
		Connected: [
			"Connected",
			"connected",
			"●",
			""
		],
		Checking: [
			"Checking…",
			"connecting",
			"◌",
			""
		],
		"Access ended": [
			"Access ended",
			"access ended",
			"✗",
			"Pair again"
		],
		"Different machine": [
			"Different machine",
			"different machine",
			"?",
			"Pair again"
		],
		"Not paired": [
			"Not paired",
			"not paired",
			"?",
			"Pair again"
		],
		"Pairing unreadable": [
			"Pairing unreadable",
			"pairing unreadable",
			"?",
			"Pair again"
		],
		Offline: [
			"Offline",
			"offline",
			"✗",
			"Retry"
		],
		Unconfirmed: [
			"Connection unconfirmed",
			"unconfirmed",
			"?",
			"Retry"
		],
		"Not accepted": [
			"Not accepted",
			"not accepted",
			"?",
			"Retry"
		],
		"Can't reach Remote": [
			"Connection unconfirmed",
			"unconfirmed",
			"?",
			"Retry"
		]
	};
	const value = Object.hasOwn(known, state) ? known[state] : void 0;
	if (!value) return {
		text: "Status unknown",
		word: "unknown",
		mark: "?",
		tone: "muted",
		action: ""
	};
	const [text, word, mark, action] = value;
	return {
		text,
		word,
		mark,
		action,
		tone: "neutral"
	};
}
/** The native page's door root, shared by the registry projection and current-row check. */
function currentDoor(pageUrl) {
	const root = new URL("./", pageUrl);
	if (root.pathname === "/remote/") root.pathname = "/";
	return root.href;
}
/** Both page owners and the menu identify the current Local row by the same door root. */
function currentRemote(entry, address, pageUrl) {
	return entry.kind === "local" && (entry.url === address || new URL(entry.url).href === currentDoor(pageUrl));
}
//#endregion
//#region src/settings-controls.ts
/** Static settings choices; values are drafts, never management authority. */
function settingsChoice(trigger, surface) {
	const options = [...surface.querySelectorAll("[role=option]")];
	const enabled = () => options.filter((option) => !option.disabled);
	const opened = () => surface.matches(":popover-open");
	function close(refocus = false) {
		if (opened()) surface.hidePopover();
		trigger.setAttribute("aria-expanded", "false");
		if (refocus && !trigger.disabled) trigger.focus({ preventScroll: true });
	}
	function place() {
		const anchor = trigger.getBoundingClientRect();
		const viewport = window.visualViewport;
		const left = viewport?.offsetLeft ?? 0;
		const top = viewport?.offsetTop ?? 0;
		const width = viewport?.width ?? document.documentElement.clientWidth;
		const height = viewport?.height ?? innerHeight;
		surface.style.maxWidth = `${Math.max(0, width - 16)}px`;
		surface.style.width = `${Math.min(anchor.width, width - 16)}px`;
		surface.style.maxHeight = `${height - 16}px`;
		const box = surface.getBoundingClientRect();
		const below = top + height - anchor.bottom - 8;
		const above = anchor.top - top - 8;
		const up = box.height > below && above > below;
		surface.style.left = `${Math.max(left + 8, Math.min(anchor.left, left + width - box.width - 8))}px`;
		surface.style.maxHeight = `${Math.max(0, up ? above : below)}px`;
		surface.style.top = `${up ? Math.max(top + 8, anchor.top - Math.min(box.height, above)) : anchor.bottom}px`;
	}
	function open(last = false) {
		if (trigger.disabled) return;
		if (!opened()) surface.showPopover();
		trigger.setAttribute("aria-expanded", "true");
		place();
		search = "";
		const items = enabled();
		(items.find((option) => option.dataset.value === trigger.value) ?? (last ? items.at(-1) : items[0]))?.focus();
	}
	let pointerOpen = false;
	let search = "";
	let searchAt = 0;
	trigger.addEventListener("pointerdown", () => {
		pointerOpen = opened();
	});
	trigger.addEventListener("click", (event) => {
		const dismiss = event.detail > 0 && pointerOpen;
		pointerOpen = false;
		if (dismiss || opened()) close();
		else open();
	});
	trigger.addEventListener("keydown", (event) => {
		if (event.key === "ArrowDown" || event.key === "ArrowUp") {
			event.preventDefault();
			open(event.key === "ArrowUp");
		}
	});
	for (const option of options) option.addEventListener("click", () => {
		if (trigger.disabled || option.disabled) return;
		trigger.value = option.dataset.value;
		close(true);
		trigger.dispatchEvent(new Event("change", { bubbles: true }));
	});
	surface.addEventListener("toggle", () => {
		trigger.setAttribute("aria-expanded", String(opened()));
	});
	surface.addEventListener("focusin", () => {
		for (const option of options) option.tabIndex = option === document.activeElement ? 0 : -1;
	});
	surface.addEventListener("keydown", (event) => {
		if (event.key === "Escape" || event.key === "Tab") {
			if (event.key === "Escape") event.preventDefault();
			close(true);
			return;
		}
		if (event.isComposing || event.altKey || event.ctrlKey || event.metaKey) return;
		const items = enabled();
		const at = items.findIndex((option) => option === document.activeElement);
		let next;
		if (event.key === "ArrowDown") next = items[(at + 1) % items.length];
		else if (event.key === "ArrowUp") next = items[(at - 1 + items.length) % items.length];
		else if (event.key === "Home") next = items[0];
		else if (event.key === "End") next = items.at(-1);
		else if (event.key.length === 1 && event.key !== " ") {
			const now = performance.now();
			search = (now - searchAt > 700 ? "" : search) + event.key.toLowerCase();
			searchAt = now;
			next = [...items.slice(at + 1), ...items.slice(0, at + 1)].find((option) => option.textContent?.trim().toLowerCase().startsWith(search));
		} else return;
		event.preventDefault();
		next?.focus();
	});
	const reposition = () => {
		if (opened()) place();
	};
	window.addEventListener("resize", reposition);
	window.addEventListener("scroll", reposition, true);
	window.visualViewport?.addEventListener("resize", reposition);
	window.visualViewport?.addEventListener("scroll", reposition);
	return () => {
		for (const option of options) {
			option.disabled = trigger.disabled || option.dataset.value === "default";
			option.setAttribute("aria-selected", String(option.dataset.value === trigger.value));
		}
		trigger.querySelector("[data-choice-value]").textContent = options.find((option) => option.dataset.value === trigger.value)?.textContent?.trim() ?? "";
		if (trigger.disabled) close();
	};
}
//#endregion
//#region src/remote-menu.ts
function remoteMenu(actions, shell = void 0) {
	if (!shell) return () => {};
	const href = (view) => new URL(remotePath(view), location.href).href;
	let projection;
	function paint() {
		const view = projection?.current.connection;
		const showing = remoteItem(location.pathname)?.id;
		const item = (id) => {
			return {
				id,
				label: REMOTE_ITEMS.find((candidate) => candidate.id === id).label,
				href: href(id),
				current: showing === id
			};
		};
		const recovery = view?.action;
		shell.setMenu({
			title: "Remote",
			groups: [{ items: [{
				...item("remotes"),
				current: [
					"remotes",
					"create",
					"join"
				].includes(showing ?? ""),
				total: projection?.rows.length
			}] }, {
				label: "This machine",
				items: [item("members"), item("my-devices")],
				action: recovery ? {
					icon: "link",
					label: recovery,
					onActivate: () => actions.recover(recovery)
				} : void 0
			}],
			settings: {
				...item("settings"),
				label: "Remote settings",
				current: showing === "settings" || showing === "service"
			}
		});
	}
	paint();
	window.addEventListener("popstate", paint, { signal: shell.signal });
	const refresh = () => {
		paint();
		actions.refresh();
	};
	const entered = (event) => {
		const menu = event.target?.closest?.(".tmt-ui-module-menu, .tmt-ui-rail-sub");
		if (menu && !menu.contains(event.relatedTarget)) refresh();
	};
	document.addEventListener("pointerover", entered, { signal: shell.signal });
	document.addEventListener("focusin", entered, { signal: shell.signal });
	return (next) => {
		projection = next;
		if (next.current.connection.word === "connected" && next.address) shell.describe(next.address);
		paint();
	};
}
//#endregion
//#region ../../../../design/browser-ui/src/overlay.ts
/** Modified, repeated and composing keys cannot dismiss an actionable layer. */
function admitsOverlayEscape(event) {
	return event.isTrusted && !event.repeat && !(event.nativeEvent?.isComposing ?? event.isComposing) && event.keyCode !== 229 && !event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey;
}
function restoreOverlayFocus(origin, fallback) {
	const usable = (node) => node instanceof HTMLElement && node.isConnected && node.getClientRects().length > 0 && !node.closest("[inert]") && !node.matches(":disabled");
	if (usable(origin)) origin.focus({ preventScroll: true });
	else {
		const node = fallback?.() ?? null;
		if (usable(node)) node.focus({ preventScroll: true });
	}
}
function outsideOverlay(node, x, y) {
	const box = node.getBoundingClientRect();
	return x < box.left || x >= box.right || y < box.top || y >= box.bottom;
}
//#endregion
//#region ../../../../design/browser-ui/src/icon-action-tooltip.ts
var activeTooltips = /* @__PURE__ */ new WeakMap();
/** Help has one ephemeral owner per document, independent of menu/dialog state. */
function claimIconActionTooltip(document, dismiss) {
	activeTooltips.get(document)?.();
	activeTooltips.set(document, dismiss);
	return () => {
		if (activeTooltips.get(document) === dismiss) activeTooltips.delete(document);
	};
}
function iconActionTooltipPosition(anchor, tooltipWidth, tooltipHeight, viewport, gutter, side) {
	const maxWidth = Math.max(0, viewport.width - gutter * 2);
	const width = Math.min(tooltipWidth, maxWidth);
	const leftEdge = viewport.left + gutter;
	const belowTop = Math.max(viewport.top, anchor.bottom);
	const below = Math.max(0, viewport.top + viewport.height - gutter - belowTop);
	const offset = side === "above" ? gutter : 0;
	const above = Math.max(0, anchor.top - viewport.top - gutter - offset);
	const flipped = side === "above" || tooltipHeight > below && above > below;
	const maxHeight = flipped ? above : below;
	const placedTop = flipped ? anchor.top - offset - Math.min(tooltipHeight, maxHeight) : belowTop;
	const top = side === "above" ? Math.max(viewport.top + gutter, placedTop) : placedTop;
	return {
		left: Math.max(leftEdge, Math.min((anchor.left + anchor.right - width) / 2, viewport.left + viewport.width - gutter - width)),
		top,
		maxWidth,
		maxHeight
	};
}
function placeIconActionTooltip(anchor, tooltip, side) {
	const view = anchor.ownerDocument.defaultView;
	if (!view) return () => {};
	const viewport = view.visualViewport;
	const place = () => {
		const box = anchor.getBoundingClientRect();
		if (!anchor.isConnected || !box.width || !box.height) {
			if (tooltip.matches(":popover-open")) tooltip.hidePopover();
			return;
		}
		const gutter = Number.parseFloat(view.getComputedStyle(tooltip).paddingTop) || 0;
		const bounds = {
			left: viewport?.offsetLeft ?? 0,
			top: viewport?.offsetTop ?? 0,
			width: viewport?.width ?? view.innerWidth,
			height: viewport?.height ?? view.innerHeight
		};
		tooltip.style.maxWidth = `${Math.max(0, bounds.width - gutter * 2)}px`;
		const tooltipBox = tooltip.getBoundingClientRect();
		const next = iconActionTooltipPosition(box, tooltipBox.width, Math.max(tooltipBox.height, tooltip.scrollHeight), bounds, gutter, side);
		tooltip.style.left = `${next.left}px`;
		tooltip.style.top = `${next.top}px`;
		tooltip.style.maxHeight = `${next.maxHeight}px`;
	};
	place();
	const observer = new ResizeObserver(place);
	observer.observe(anchor);
	observer.observe(tooltip);
	view.addEventListener("resize", place);
	view.addEventListener("scroll", place, true);
	viewport?.addEventListener("resize", place);
	viewport?.addEventListener("scroll", place);
	return () => {
		observer.disconnect();
		view.removeEventListener("resize", place);
		view.removeEventListener("scroll", place, true);
		viewport?.removeEventListener("resize", place);
		viewport?.removeEventListener("scroll", place);
	};
}
var escapeFocus = /* @__PURE__ */ new WeakSet();
function guardEscapeFocus(document) {
	escapeFocus.add(document);
	const clear = () => {
		escapeFocus.delete(document);
		document.removeEventListener("keydown", nextKey, true);
	};
	const nextKey = (event) => {
		if (event.key !== "Escape") clear();
	};
	document.addEventListener("keydown", nextKey, true);
	document.defaultView?.requestAnimationFrame(clear);
}
/** Internal binding shared by icon renderers; activation and its busy fence stay with the caller. */
function bindIconActionTooltip(wrapper, button, tooltip, placement) {
	const document = button.ownerDocument;
	const view = document.defaultView;
	let hovered = false;
	let focused = false;
	let dismissed = false;
	let disposed = false;
	let expanded = button.getAttribute("aria-expanded") === "true";
	let returnGuard = false;
	let returnFrame;
	let release;
	let position;
	const close = () => {
		position?.();
		position = void 0;
		release?.();
		release = void 0;
		document.removeEventListener("keydown", escape, true);
		if (tooltip.matches(":popover-open")) tooltip.hidePopover();
	};
	const dismiss = () => {
		dismissed = true;
		close();
	};
	const escape = (event) => {
		if (event.key !== "Escape" || !tooltip.matches(":popover-open")) return;
		dismiss();
		guardEscapeFocus(document);
	};
	const paint = () => {
		if (!(!disposed && (hovered || focused) && !dismissed && !expanded && button.isConnected && button.getClientRects().length > 0)) {
			close();
			return;
		}
		if (tooltip.matches(":popover-open")) return;
		const anchor = placement ? placement.anchor() : button;
		if (!anchor?.isConnected) return;
		release = claimIconActionTooltip(document, dismiss);
		tooltip.showPopover();
		position = placeIconActionTooltip(anchor, tooltip, placement?.side);
		document.addEventListener("keydown", escape, true);
	};
	const clearReturnGuard = () => {
		returnGuard = false;
		document.removeEventListener("keydown", returnKey, true);
		if (returnFrame !== void 0) view.cancelAnimationFrame(returnFrame);
		returnFrame = void 0;
	};
	const returnKey = (event) => {
		if (event.key !== "Escape") clearReturnGuard();
	};
	const refresh = () => {
		const next = button.getAttribute("aria-expanded") === "true";
		if (expanded && !next) {
			clearReturnGuard();
			returnGuard = true;
			document.addEventListener("keydown", returnKey, true);
			returnFrame = view.requestAnimationFrame(clearReturnGuard);
		}
		expanded = next;
		if (expanded) dismissed = true;
		paint();
	};
	const move = () => {
		if (!hovered) {
			hovered = true;
			dismissed = false;
		}
		refresh();
	};
	const leave = () => {
		hovered = false;
		refresh();
	};
	const focus = () => {
		if (!button.matches(":focus-visible")) return;
		focused = true;
		expanded = button.getAttribute("aria-expanded") === "true";
		dismissed = expanded || returnGuard || escapeFocus.has(document);
		paint();
	};
	const blur = () => {
		focused = false;
		refresh();
	};
	wrapper.addEventListener("pointermove", move);
	wrapper.addEventListener("pointerleave", leave);
	button.addEventListener("focus", focus);
	button.addEventListener("blur", blur);
	return {
		refresh,
		dismiss,
		dispose() {
			if (disposed) return;
			disposed = true;
			close();
			clearReturnGuard();
			wrapper.removeEventListener("pointermove", move);
			wrapper.removeEventListener("pointerleave", leave);
			button.removeEventListener("focus", focus);
			button.removeEventListener("blur", blur);
		}
	};
}
//#endregion
//#region ../../../../design/browser-ui/src/menu-position.ts
/** Menus share the leaf's below/flip/clamp rule, aligning to the trigger's end. */
function browserMenuPosition(anchor, width, height, viewport, gutter = 8) {
	const placed = iconActionTooltipPosition(anchor, width, height, viewport, gutter);
	return {
		...placed,
		left: Math.max(viewport.left + gutter, Math.min(anchor.right - Math.min(width, placed.maxWidth), viewport.left + viewport.width - gutter - Math.min(width, placed.maxWidth)))
	};
}
//#endregion
//#region ../../../../design/browser-ui/src/popup.ts
var activePopups = /* @__PURE__ */ new WeakMap();
/** Menu and combobox surfaces share one replacement owner, without owning host commands. */
function claimBrowserPopup(document, dismiss) {
	activePopups.get(document)?.();
	activePopups.set(document, dismiss);
	return () => {
		if (activePopups.get(document) === dismiss) activePopups.delete(document);
	};
}
function placeBrowserPopup(anchor, surface, dismiss) {
	const view = anchor.ownerDocument.defaultView;
	const viewport = view.visualViewport;
	const place = () => {
		if (!anchor.isConnected || !anchor.getClientRects().length) return dismiss();
		const bounds = {
			left: viewport?.offsetLeft ?? 0,
			top: viewport?.offsetTop ?? 0,
			width: viewport?.width ?? view.innerWidth,
			height: viewport?.height ?? view.innerHeight
		};
		surface.style.maxWidth = `${Math.max(0, bounds.width - 16)}px`;
		const box = surface.getBoundingClientRect();
		const position = browserMenuPosition(anchor.getBoundingClientRect(), box.width, Math.max(box.height, surface.scrollHeight + surface.offsetHeight - surface.clientHeight), bounds);
		Object.assign(surface.style, {
			left: `${position.left}px`,
			top: `${position.top}px`,
			maxHeight: `${position.maxHeight}px`
		});
	};
	place();
	let frame;
	const observer = new ResizeObserver(() => {
		if (frame !== void 0) return;
		frame = view.requestAnimationFrame(() => {
			frame = void 0;
			place();
		});
	});
	observer.observe(anchor);
	observer.observe(surface);
	view.addEventListener("resize", place);
	view.addEventListener("scroll", place, true);
	viewport?.addEventListener("resize", place);
	viewport?.addEventListener("scroll", place);
	return () => {
		observer.disconnect();
		if (frame !== void 0) view.cancelAnimationFrame(frame);
		view.removeEventListener("resize", place);
		view.removeEventListener("scroll", place, true);
		viewport?.removeEventListener("resize", place);
		viewport?.removeEventListener("scroll", place);
	};
}
//#endregion
//#region ../../../../design/browser-ui/src/menu-controller.ts
/** Segment items without changing the flat controller order. Unheaded leading items remain ungrouped. */
function menuGroups(items) {
	const groups = [];
	items.forEach((item, index) => {
		if (item.heading || groups.length === 0) groups.push({
			heading: item.heading,
			entries: []
		});
		groups[groups.length - 1].entries.push({
			item,
			index
		});
	});
	return groups;
}
/** One transient menu owner for React and framework-free renderers; commands stay with callers. */
function createMenuController(options) {
	let snapshot = {
		open: false,
		present: false
	};
	const listeners = /* @__PURE__ */ new Set();
	let first = "first";
	let search = {
		text: "",
		at: 0
	};
	let focused;
	let reported = false;
	let disposed = false;
	let detach;
	let cancelExit;
	const enabled = () => options.items().flatMap((item, index) => item.disabled ? [] : [index]);
	const publish = (open, present) => {
		if (snapshot.open === open && snapshot.present === present) return;
		snapshot = {
			open,
			present
		};
		for (const notify of listeners) notify();
	};
	const report = (open) => {
		if (reported === open) return;
		reported = open;
		options.onOpenChange(open);
	};
	const canOpen = () => !disposed && !options.disabled() && (enabled().length > 0 || options.hasDescription());
	function open(edge = "first") {
		if (!canOpen() || snapshot.open) return;
		cancelExit?.();
		cancelExit = void 0;
		first = edge;
		focused = void 0;
		search = {
			text: "",
			at: 0
		};
		publish(true, true);
	}
	function close(refocus = true) {
		if (!snapshot.open) return;
		const node = options.surface();
		publish(false, snapshot.present);
		if (node) {
			node.dataset.state = "closed";
			if (node.open) node.close();
		}
		detach?.();
		report(false);
		if (refocus) restoreOverlayFocus(options.trigger());
		const animations = node?.getAnimations?.() ?? [];
		if (!animations.length) {
			publish(false, false);
			return;
		}
		let current = true;
		cancelExit = () => {
			current = false;
		};
		Promise.allSettled(animations.map((animation) => animation.finished)).then(() => {
			if (current && !snapshot.open && !disposed) {
				cancelExit = void 0;
				publish(false, false);
			}
		});
	}
	function attachSurface() {
		const node = options.surface();
		const anchor = options.trigger();
		if (!snapshot.open || !node || !anchor) return;
		if (!node.isConnected || !anchor.isConnected || !anchor.getClientRects().length) {
			close(false);
			return;
		}
		if (detach) return detach;
		const release = claimBrowserPopup(anchor.ownerDocument, () => close(false));
		let position;
		const cleanup = () => {
			if (detach !== cleanup) return;
			detach = void 0;
			position?.();
			release();
			if (node.open) node.close();
			queueMicrotask(() => {
				const current = options.surface();
				if (!node.isConnected && snapshot.open && (!current || current === node)) close(false);
			});
		};
		detach = cleanup;
		try {
			node.showModal();
			position = placeBrowserPopup(anchor, node, () => close(false));
			if (!snapshot.open) return cleanup;
			report(true);
			if (!snapshot.open) return cleanup;
			const choices = enabled();
			const index = first === "last" ? choices.at(-1) : choices[0];
			focused = index === void 0 ? void 0 : options.items()[index]?.key;
			(index === void 0 ? node : options.entries()[index] ?? node).focus();
		} catch (error) {
			cleanup();
			close(false);
			throw error;
		}
		return cleanup;
	}
	function refresh() {
		const node = options.surface();
		if (!snapshot.open || !node) return;
		const active = node.ownerDocument.activeElement;
		if (active === node) return;
		if (active !== node.ownerDocument.body && (!active || !node.contains(active))) return;
		const entries = options.entries();
		const choices = enabled();
		if (choices.some((index) => entries[index] === active)) return;
		const next = choices.find((index) => options.items()[index].key === focused) ?? choices[0];
		(next === void 0 ? node : entries[next] ?? node).focus();
	}
	return {
		getSnapshot: () => snapshot,
		subscribe(notify) {
			listeners.add(notify);
			return () => {
				listeners.delete(notify);
			};
		},
		canOpen,
		open,
		close,
		attachSurface,
		refresh,
		focused(key) {
			if (snapshot.open) focused = key;
		},
		triggerKey(event) {
			if (!["ArrowDown", "ArrowUp"].includes(event.key) || !event.isTrusted || (event.nativeEvent?.isComposing ?? event.isComposing)) return;
			event.preventDefault();
			open(event.key === "ArrowUp" ? "last" : "first");
		},
		menuKey(event) {
			if (!snapshot.open || event.defaultPrevented) return;
			if (event.key === "Escape") {
				event.preventDefault();
				event.stopPropagation();
				if (admitsOverlayEscape(event)) close();
				return;
			}
			if ((event.nativeEvent?.isComposing ?? event.isComposing) || event.altKey || event.ctrlKey || event.metaKey) return;
			if (event.key === "Tab") {
				close();
				return;
			}
			const choices = enabled();
			const entries = options.entries();
			const at = choices.indexOf(entries.findIndex((node) => node === node?.ownerDocument.activeElement));
			let next;
			if (event.key === "ArrowDown") next = choices[(at + 1) % choices.length];
			else if (event.key === "ArrowUp") next = choices[(at - 1 + choices.length) % choices.length];
			else if (event.key === "Home") next = choices[0];
			else if (event.key === "End") next = choices.at(-1);
			else if (event.key.length === 1 && event.key !== " ") {
				const now = performance.now();
				search = {
					text: (now - search.at > 700 ? "" : search.text) + event.key.toLocaleLowerCase(),
					at: now
				};
				next = [...choices.slice(at + 1), ...choices.slice(0, at + 1)].find((index) => options.items()[index].label.toLocaleLowerCase().startsWith(search.text));
			} else return;
			event.preventDefault();
			if (next !== void 0) {
				focused = options.items()[next]?.key;
				entries[next]?.focus();
			}
		},
		select(key, event) {
			const item = options.items().find((item) => item.key === key);
			if (!snapshot.open || !event.isTrusted || !item || item.disabled) return;
			const select = options.selection();
			close();
			select(key, event);
		},
		nativeClose(event) {
			event.stopPropagation();
			const node = options.surface();
			if (snapshot.open && event.target === node && !node?.open) close();
		},
		cancel(event) {
			event.preventDefault();
			event.stopPropagation();
			if (event.isTrusted) close();
		},
		pointerDown(event) {
			const node = options.surface();
			if (node && event.target === node && outsideOverlay(node, event.clientX, event.clientY)) event.preventDefault();
		},
		surfaceClick(event) {
			const node = options.surface();
			if (!node || event.target !== node || !outsideOverlay(node, event.clientX, event.clientY)) return;
			event.preventDefault();
			event.stopPropagation();
			if (event.isTrusted) close();
		},
		dispose() {
			if (disposed) return;
			close(false);
			detach?.();
			cancelExit?.();
			cancelExit = void 0;
			disposed = true;
			publish(false, false);
			listeners.clear();
		}
	};
}
//#endregion
//#region ../../../../design/browser-ui/src/shell-icons.ts
/**
* Lucide 1.52.0 (ISC, see `LICENSE.lucide`) node data for the shell, so static hosts draw the
* same icons without a React or Lucide runtime. The module icon allow-list below is the subset
* of Remote's describe contract; an unknown name falls back to `box`.
*/
var shellIconNodes = {
	list: [["path", { d: "M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01" }]],
	bookmark: [["path", { d: "m19 21-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z" }]],
	plus: [["path", { d: "M12 5v14M5 12h14" }]],
	"grip-vertical": [
		["circle", {
			cx: "9",
			cy: "12",
			r: "1"
		}],
		["circle", {
			cx: "9",
			cy: "5",
			r: "1"
		}],
		["circle", {
			cx: "9",
			cy: "19",
			r: "1"
		}],
		["circle", {
			cx: "15",
			cy: "12",
			r: "1"
		}],
		["circle", {
			cx: "15",
			cy: "5",
			r: "1"
		}],
		["circle", {
			cx: "15",
			cy: "19",
			r: "1"
		}]
	],
	pencil: [["path", { d: "M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z" }], ["path", { d: "m15 5 4 4" }]],
	"trash-2": [
		["path", { d: "M10 11v6" }],
		["path", { d: "M14 11v6" }],
		["path", { d: "M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" }],
		["path", { d: "M3 6h18" }],
		["path", { d: "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" }]
	],
	link: [["path", { d: "M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" }], ["path", { d: "M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" }]],
	info: [
		["circle", {
			cx: "12",
			cy: "12",
			r: "10"
		}],
		["path", { d: "M12 16v-4" }],
		["path", { d: "M12 8h.01" }]
	],
	sun: [["circle", {
		cx: "12",
		cy: "12",
		r: "4"
	}], ["path", { d: "M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.5 1.5m11 11L19 19M5 19l1.5-1.5m11-11L19 5" }]],
	house: [["path", { d: "M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8" }], ["path", { d: "M3 10a2 2 0 0 1 .709-1.528l7-6a2 2 0 0 1 2.582 0l7 6A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" }]],
	"file-text": [
		["path", { d: "M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z" }],
		["path", { d: "M14 2v5a1 1 0 0 0 1 1h5" }],
		["path", { d: "M10 9H8" }],
		["path", { d: "M16 13H8" }],
		["path", { d: "M16 17H8" }]
	],
	"monitor-smartphone": [
		["path", { d: "M18 8V6a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h8" }],
		["path", { d: "M10 19v-3.96 3.15" }],
		["path", { d: "M7 19h5" }],
		["rect", {
			width: "6",
			height: "10",
			x: "16",
			y: "12",
			rx: "2"
		}]
	],
	user: [["path", { d: "M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2" }], ["circle", {
		cx: "12",
		cy: "7",
		r: "4"
	}]],
	users: [
		["path", { d: "M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" }],
		["path", { d: "M16 3.128a4 4 0 0 1 0 7.744" }],
		["path", { d: "M22 21v-2a4 4 0 0 0-3-3.87" }],
		["circle", {
			cx: "9",
			cy: "7",
			r: "4"
		}]
	],
	"chart-column": [
		["path", { d: "M3 3v16a2 2 0 0 0 2 2h16" }],
		["path", { d: "M18 17V9" }],
		["path", { d: "M13 17V5" }],
		["path", { d: "M8 17v-3" }]
	],
	search: [["path", { d: "m21 21-4.34-4.34" }], ["circle", {
		cx: "11",
		cy: "11",
		r: "8"
	}]],
	settings: [["path", { d: "M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915" }], ["circle", {
		cx: "12",
		cy: "12",
		r: "3"
	}]],
	box: [
		["path", { d: "M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z" }],
		["path", { d: "m3.3 7 8.7 5 8.7-5" }],
		["path", { d: "M12 22V12" }]
	],
	menu: [
		["path", { d: "M4 5h16" }],
		["path", { d: "M4 12h16" }],
		["path", { d: "M4 19h16" }]
	],
	"chevron-down": [["path", { d: "m6 9 6 6 6-6" }]],
	"chevron-right": [["path", { d: "m9 18 6-6-6-6" }]],
	"triangle-alert": [
		["path", { d: "m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3" }],
		["path", { d: "M12 9v4" }],
		["path", { d: "M12 17h.01" }]
	],
	lock: [["rect", {
		width: "18",
		height: "11",
		x: "3",
		y: "11",
		rx: "2",
		ry: "2"
	}], ["path", { d: "M7 11V7a5 5 0 0 1 10 0v4" }]],
	clock: [["circle", {
		cx: "12",
		cy: "12",
		r: "10"
	}], ["path", { d: "M12 6v6l4 2" }]],
	terminal: [["path", { d: "M12 19h8" }], ["path", { d: "m4 17 6-6-6-6" }]],
	ellipsis: [
		["circle", {
			cx: "12",
			cy: "12",
			r: "1"
		}],
		["circle", {
			cx: "19",
			cy: "12",
			r: "1"
		}],
		["circle", {
			cx: "5",
			cy: "12",
			r: "1"
		}]
	],
	copy: [["rect", {
		width: "14",
		height: "14",
		x: "8",
		y: "8",
		rx: "2",
		ry: "2"
	}], ["path", { d: "M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" }]],
	x: [["path", { d: "M18 6 6 18" }], ["path", { d: "m6 6 12 12" }]]
};
//#endregion
//#region ../../../../design/browser-ui/src/static.ts
var browserUiClasses = Object.freeze({
	chips: "tmt-ui-chips",
	chip: "tmt-ui-chip",
	chipLabel: "tmt-ui-chip-label",
	chipsAdd: "tmt-ui-chips-add",
	chipsStatus: "tmt-ui-chips-status",
	chipsSurface: "tmt-ui-chips-surface",
	chipsSearch: "tmt-ui-chips-search",
	chipsOption: "tmt-ui-chips-option",
	countChip: "tmt-ui-count-chip",
	count: "tmt-ui-count",
	avatar: "tmt-ui-avatar",
	radio: "tmt-ui-radio",
	choices: "tmt-ui-choices",
	choicesLegend: "tmt-ui-choices-legend",
	choice: "tmt-ui-choice",
	choiceText: "tmt-ui-choice-text",
	choiceDescription: "tmt-ui-choice-description",
	choiceHint: "tmt-ui-choice-hint",
	pager: "tmt-ui-pager",
	pagerPosition: "tmt-ui-pager-position",
	statusTiles: "tmt-ui-status-tiles",
	statusTile: "tmt-ui-status-tile",
	statusTileLabel: "tmt-ui-status-tile-label",
	statusTileValue: "tmt-ui-status-tile-value",
	statusTileDot: "tmt-ui-status-tile-dot",
	stateMark: "tmt-ui-state-mark",
	card: "tmt-ui-card",
	cardHead: "tmt-ui-card-head",
	cardTitle: "tmt-ui-card-title",
	cardMeta: "tmt-ui-card-meta",
	meter: "tmt-ui-meter",
	meterFill: "tmt-ui-meter-fill",
	glyph: "tmt-ui-glyph",
	page: "tmt-ui-page",
	pageHead: "tmt-ui-page-head",
	pageTitle: "tmt-ui-page-title",
	topBarTitlePrefix: "tmt-ui-topbar-title-prefix",
	topBarTitleCurrent: "tmt-ui-topbar-title-current",
	topBarTitleChips: "tmt-ui-topbar-title-chips",
	pageSubtitle: "tmt-ui-page-subtitle",
	stats: "tmt-ui-stats",
	stat: "tmt-ui-stat",
	statValue: "tmt-ui-stat-value",
	statNumber: "tmt-ui-stat-number",
	statCaption: "tmt-ui-stat-caption",
	banner: "tmt-ui-banner",
	bannerHead: "tmt-ui-banner-head",
	bannerRow: "tmt-ui-banner-row",
	facts: "tmt-ui-facts",
	tableScroll: "tmt-ui-table-scroll",
	tableGroup: "tmt-ui-table-group",
	tableGroupToggle: "tmt-ui-table-group-toggle",
	tableGroupCount: "tmt-ui-table-group-count",
	tableSelect: "tmt-ui-table-select",
	tableCompactMeta: "tmt-ui-table-compact-meta",
	tableMono: "tmt-ui-table-mono",
	tableQuiet: "tmt-ui-table-quiet",
	avatarStack: "tmt-ui-avatar-stack",
	glyphTile: "tmt-ui-glyph-tile",
	notifications: "tmt-ui-notifications",
	notification: "tmt-ui-notification",
	notificationStatus: "tmt-ui-notification-status",
	notificationHeading: "tmt-ui-notification-heading",
	notificationTag: "tmt-ui-notification-tag",
	notificationLine: "tmt-ui-notification-line",
	notificationQuote: "tmt-ui-notification-quote",
	notificationWaiting: "tmt-ui-notification-waiting",
	drawer: "tmt-ui-drawer",
	drawerHeader: "tmt-ui-drawer-header",
	drawerBody: "tmt-ui-drawer-body",
	dialog: "tmt-ui-dialog",
	dialogHeader: "tmt-ui-dialog-header",
	dialogBody: "tmt-ui-dialog-body",
	dialogActions: "tmt-ui-dialog-actions",
	menu: "tmt-ui-menu",
	menuSurface: "tmt-ui-menu-surface",
	menuItem: "tmt-ui-menu-item",
	command: "tmt-ui-command",
	commandText: "tmt-ui-command-text",
	commandCopy: "tmt-ui-command-copy",
	commandFeedback: "tmt-ui-command-feedback",
	code: "tmt-ui-code",
	header: "tmt-ui-header",
	brand: "tmt-ui-brand",
	mark: "tmt-ui-mark",
	wordmark: "tmt-ui-wordmark",
	heading: "tmt-ui-heading",
	headingSuffix: "tmt-ui-heading-suffix",
	title: "tmt-ui-title",
	caption: "tmt-ui-caption",
	status: "tmt-ui-status",
	actions: "tmt-ui-actions",
	icon: "tmt-ui-icon",
	notice: "tmt-ui-notice",
	noticeMark: "tmt-ui-notice-mark",
	noticeEyebrow: "tmt-ui-notice-eyebrow",
	noticeHeading: "tmt-ui-notice-heading",
	noticeBody: "tmt-ui-notice-body",
	noticeActions: "tmt-ui-notice-actions",
	field: "tmt-ui-field",
	fieldLabel: "tmt-ui-field-label",
	fieldControl: "tmt-ui-field-control",
	searchField: "tmt-ui-search-field",
	fieldDescription: "tmt-ui-field-description",
	fieldError: "tmt-ui-field-error",
	action: "tmt-ui-action",
	actionLabel: "tmt-ui-action-label",
	actionMark: "tmt-ui-action-mark",
	busyMark: "tmt-ui-busy-mark",
	busyDot: "tmt-ui-busy-dot",
	checkbox: "tmt-ui-checkbox",
	checkboxLabel: "tmt-ui-checkbox-label",
	iconAction: "tmt-ui-icon-action",
	iconActionControl: "tmt-ui-icon-action-control",
	iconActionIcon: "tmt-ui-icon-action-icon",
	iconActionTooltip: "tmt-ui-icon-action-tooltip",
	iconActionTooltipLabel: "tmt-ui-icon-action-tooltip-label",
	toggle: "tmt-ui-toggle",
	toggleIndicator: "tmt-ui-toggle-indicator",
	list: "tmt-ui-list",
	listRow: "tmt-ui-list-row",
	listLeading: "tmt-ui-list-leading",
	listTitle: "tmt-ui-list-title",
	listMeta: "tmt-ui-list-meta",
	listLabel: "tmt-ui-list-label",
	listDescription: "tmt-ui-list-description",
	listUpdated: "tmt-ui-list-updated",
	listState: "tmt-ui-list-state",
	listActions: "tmt-ui-list-actions",
	tabs: "tmt-ui-tabs",
	tabsHeader: "tmt-ui-tabs-header",
	tabList: "tmt-ui-tab-list",
	tab: "tmt-ui-tab",
	tabMark: "tmt-ui-tab-mark",
	tabLabel: "tmt-ui-tab-label",
	tabCount: "tmt-ui-tab-count",
	tabsStatus: "tmt-ui-tabs-status",
	tabPanel: "tmt-ui-tab-panel",
	section: "tmt-ui-section",
	sectionHeading: "tmt-ui-section-heading",
	table: "tmt-ui-table",
	disclosure: "tmt-ui-disclosure",
	visuallyHidden: "tmt-ui-visually-hidden"
});
/**
* The browser busy mark's six dots on a 16x16 viewBox, in ring order (clockwise from the top
* left of the 2x3 cell). Each dot's eased lap is offset by its place in the ring in CSS.
*/
var busyDots = [
	[5, 3],
	[11, 3],
	[11, 8],
	[11, 13],
	[5, 13],
	[5, 8]
];
/** The radius of each busy-mark dot on the 16x16 viewBox. */
var busyDotRadius = 1.6;
var SVG$1 = "http://www.w3.org/2000/svg";
/** The busy mark's SVG (its dots in currentColor), as `BusyMark` renders it. */
function busySvg(doc) {
	const svg = doc.createElementNS(SVG$1, "svg");
	svg.setAttribute("viewBox", "0 0 16 16");
	for (const [cx, cy] of busyDots) {
		const dot = doc.createElementNS(SVG$1, "circle");
		dot.setAttribute("class", browserUiClasses.busyDot);
		dot.setAttribute("cx", String(cx));
		dot.setAttribute("cy", String(cy));
		dot.setAttribute("r", String(busyDotRadius));
		svg.append(dot);
	}
	return svg;
}
/** The busy mark alone, for a framework-free wait such as a route that is still loading. */
function createBusyMark(doc = document) {
	const busy = doc.createElement("span");
	busy.className = browserUiClasses.busyMark;
	busy.append(busySvg(doc));
	return busy;
}
function createActionMark(doc = document) {
	const mark = doc.createElement("span");
	mark.className = browserUiClasses.actionMark;
	mark.setAttribute("aria-hidden", "true");
	mark.dataset.busy = "false";
	mark.append(createBusyMark(doc));
	return mark;
}
//#endregion
//#region ../../../../design/browser-ui/src/glyph.ts
var SVG = "http://www.w3.org/2000/svg";
/** A line icon from the shared icon nodes, for a framework-free page: it takes the text colour. */
function createIcon(name, doc = document) {
	const svg = doc.createElementNS(SVG, "svg");
	svg.setAttribute("class", browserUiClasses.glyph);
	svg.setAttribute("viewBox", "0 0 24 24");
	svg.setAttribute("aria-hidden", "true");
	for (const [tag, attributes] of shellIconNodes[name]) {
		const node = doc.createElementNS(SVG, tag);
		for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
		svg.append(node);
	}
	return svg;
}
//#endregion
//#region ../../../../design/browser-ui/src/menu-host.ts
/** Mount the existing menu presentation without a framework; hosts retain values and commands. */
function mountBrowserMenu(container, initial) {
	const document = container.ownerDocument;
	const id = `tmt-menu-${document.defaultView.crypto.randomUUID()}`;
	const root = document.createElement("span");
	root.className = "tmt-ui-menu";
	const wrapper = document.createElement("span");
	const trigger = document.createElement("button");
	trigger.type = "button";
	trigger.setAttribute("aria-haspopup", "menu");
	const tooltip = document.createElement("span");
	tooltip.className = browserUiClasses.iconActionTooltip;
	tooltip.setAttribute("popover", "manual");
	tooltip.setAttribute("aria-hidden", "true");
	const tooltipLabel = document.createElement("span");
	tooltipLabel.className = browserUiClasses.iconActionTooltipLabel;
	tooltip.append(tooltipLabel);
	wrapper.append(trigger, tooltip);
	const surface = document.createElement("dialog");
	surface.className = "tmt-ui-menu-surface";
	surface.tabIndex = -1;
	surface.hidden = true;
	const menu = document.createElement("div");
	menu.id = id;
	menu.setAttribute("role", "menu");
	menu.tabIndex = -1;
	const description = document.createElement("div");
	description.id = `${id}-description`;
	description.className = "tmt-ui-menu-description";
	surface.append(menu);
	root.append(wrapper, surface);
	let options = initial;
	let disposed = false;
	let help;
	const rows = /* @__PURE__ */ new Map();
	const groups = /* @__PURE__ */ new Map();
	function syncChildren(parent, children) {
		for (const [index, child] of children.entries()) if (parent.childNodes[index] !== child) parent.insertBefore(child, parent.childNodes[index] ?? null);
		while (parent.childNodes.length > children.length) parent.lastChild.remove();
	}
	const controller = createMenuController({
		items: () => options.items,
		disabled: () => options.disabled === true,
		hasDescription: () => Boolean(options.description),
		trigger: () => trigger,
		surface: () => surface,
		entries: () => options.items.map((item) => rows.get(item.key)?.button ?? null),
		selection: () => options.onSelect,
		onOpenChange: (open) => options.onOpenChange?.(open)
	});
	const display = (target, content) => target.replaceChildren(typeof content === "string" ? document.createTextNode(content) : content);
	const validate = (next) => {
		if (!next.label.trim()) throw new Error("Menu requires a nonempty label");
		if (new Set(next.items.map((item) => item.key)).size !== next.items.length) throw new Error("Menu item keys must be unique");
	};
	function renderOptions() {
		const iconOnly = options.triggerText === void 0;
		if (!iconOnly && options.presentation) root.dataset.presentation = options.presentation;
		else delete root.dataset.presentation;
		wrapper.className = iconOnly ? browserUiClasses.iconAction : "tmt-ui-menu-trigger-wrapper";
		trigger.className = iconOnly ? `${browserUiClasses.action} ${browserUiClasses.iconActionControl}` : "tmt-ui-menu-trigger";
		if (iconOnly) trigger.dataset.variant = "text";
		else delete trigger.dataset.variant;
		trigger.setAttribute("aria-label", options.label);
		tooltipLabel.textContent = options.label;
		tooltip.hidden = !iconOnly;
		surface.setAttribute("aria-label", options.label);
		if (options.popupLayout === "grouped") surface.dataset.popupLayout = "grouped";
		else delete surface.dataset.popupLayout;
		menu.setAttribute("aria-label", options.label);
		trigger.disabled = !controller.canOpen();
		if (iconOnly) {
			const icon = document.createElement("span");
			icon.className = browserUiClasses.iconActionIcon;
			icon.setAttribute("aria-hidden", "true");
			icon.append(options.icon ?? createIcon("ellipsis", document));
			trigger.replaceChildren(icon);
			help ??= bindIconActionTooltip(wrapper, trigger, tooltip);
		} else {
			help?.dispose();
			help = void 0;
			const label = document.createElement("span");
			label.className = "tmt-ui-menu-value";
			label.textContent = options.triggerText;
			trigger.replaceChildren(label, createIcon("chevron-down", document));
		}
		if (options.description) {
			display(description, options.description);
			menu.setAttribute("aria-describedby", description.id);
		} else menu.removeAttribute("aria-describedby");
		const removed = new Set(rows.keys());
		const children = options.description ? [description] : [];
		for (const [index, item] of options.items.entries()) {
			let row = rows.get(item.key);
			if (!row) {
				const button = document.createElement("button");
				button.type = "button";
				button.className = "tmt-ui-menu-item";
				button.tabIndex = -1;
				button.onclick = (event) => controller.select(item.key, event);
				button.onfocus = () => controller.focused(item.key);
				row = {
					root: document.createElement("div"),
					button,
					label: document.createElement("span"),
					check: document.createElement("span"),
					reason: document.createElement("span")
				};
				row.check.setAttribute("aria-hidden", "true");
				row.check.textContent = "✓";
				row.reason.className = "tmt-ui-menu-reason";
				row.root.append(button);
				rows.set(item.key, row);
			}
			removed.delete(item.key);
			display(row.label, item.content ?? item.label);
			if (item.leadingIcon || item.detail || item.meta !== void 0) {
				const parts = [];
				if (item.leadingIcon) {
					const icon = document.createElement("span");
					icon.className = "tmt-ui-menu-leading-icon";
					icon.setAttribute("aria-hidden", "true");
					icon.append(item.leadingIcon);
					parts.push(icon);
				}
				const label = document.createElement("span");
				label.className = "tmt-ui-menu-label";
				label.append(row.label);
				if (item.detail) {
					const detail = document.createElement("span");
					detail.className = "tmt-ui-menu-detail";
					detail.id = `${id}-detail-${index}`;
					detail.textContent = item.detail;
					label.append(detail);
				}
				parts.push(label);
				if (item.meta !== void 0) {
					const meta = document.createElement("span");
					meta.className = "tmt-ui-menu-meta";
					meta.textContent = item.meta;
					parts.push(meta);
				}
				if (item.checked) parts.push(row.check);
				row.button.replaceChildren(...parts);
				row.button.setAttribute("aria-label", [item.label, item.meta].filter(Boolean).join(" "));
			} else {
				row.button.replaceChildren(row.label, ...item.checked ? [row.check] : []);
				row.button.removeAttribute("aria-label");
			}
			row.button.disabled = item.disabled === true;
			row.button.setAttribute("role", item.role ?? (item.checked === void 0 ? "menuitem" : "menuitemradio"));
			if (item.checked === void 0) row.button.removeAttribute("aria-checked");
			else row.button.setAttribute("aria-checked", String(item.checked));
			if (item.separated && !item.heading) row.button.dataset.separated = "true";
			else delete row.button.dataset.separated;
			if (item.disabledReason) {
				row.reason.id = `${id}-reason-${index}`;
				row.reason.textContent = item.disabledReason;
				if (!row.reason.isConnected) row.root.append(row.reason);
			} else row.reason.remove();
			const described = [item.detail ? `${id}-detail-${index}` : void 0, item.disabledReason ? row.reason.id : void 0].filter(Boolean);
			if (described.length) row.button.setAttribute("aria-describedby", described.join(" "));
			else row.button.removeAttribute("aria-describedby");
			children.push(row.root);
		}
		for (const key of removed) {
			rows.get(key).root.remove();
			rows.delete(key);
		}
		if (options.items.some((item) => item.heading)) {
			const output = options.description ? [description] : [];
			const used = /* @__PURE__ */ new Set();
			for (const [index, group] of menuGroups(options.items).entries()) {
				if (!group.heading) {
					output.push(...group.entries.map(({ item }) => rows.get(item.key).root));
					continue;
				}
				const first = group.entries[0].item;
				let node = groups.get(first.key);
				if (!node) {
					node = {
						root: document.createElement("div"),
						heading: document.createElement("div")
					};
					node.root.setAttribute("role", "group");
					node.heading.className = "tmt-ui-menu-heading";
					groups.set(first.key, node);
				}
				used.add(first.key);
				node.heading.id = `${id}-group-${index}`;
				node.heading.textContent = group.heading;
				if (first.separated) node.heading.dataset.separated = "true";
				else delete node.heading.dataset.separated;
				node.root.setAttribute("aria-labelledby", node.heading.id);
				syncChildren(node.root, [node.heading, ...group.entries.map(({ item }) => rows.get(item.key).root)]);
				output.push(node.root);
			}
			for (const key of groups.keys()) if (!used.has(key)) groups.delete(key);
			syncChildren(menu, output);
		} else {
			groups.clear();
			syncChildren(menu, children);
		}
		controller.refresh();
		help?.refresh();
	}
	function renderState() {
		const { open, present } = controller.getSnapshot();
		trigger.setAttribute("aria-expanded", String(open));
		if (open) trigger.setAttribute("aria-controls", id);
		else trigger.removeAttribute("aria-controls");
		surface.hidden = !present;
		surface.dataset.state = open ? "open" : "closed";
		help?.refresh();
		if (open) controller.attachSurface();
	}
	trigger.onclick = (event) => {
		if (event.isTrusted) {
			help?.dismiss();
			controller.open();
		}
	};
	trigger.onkeydown = controller.triggerKey;
	surface.onkeydown = controller.menuKey;
	surface.onclose = controller.nativeClose;
	surface.oncancel = controller.cancel;
	surface.onpointerdown = controller.pointerDown;
	surface.onclick = controller.surfaceClick;
	validate(initial);
	container.append(root);
	renderOptions();
	const unsubscribe = controller.subscribe(renderState);
	renderState();
	return {
		update(next) {
			if (!disposed) {
				validate(next);
				options = next;
				renderOptions();
				renderState();
			}
		},
		open: () => controller.open(),
		close: () => controller.close(),
		dispose() {
			if (disposed) return;
			disposed = true;
			controller.dispose();
			unsubscribe();
			help?.dispose();
			root.remove();
			rows.clear();
		}
	};
}
//#endregion
//#region ../../../../design/browser-ui/src/list-table-host.ts
/** Framework-free owner for the same table presentation used by the React entry. */
function createListTable(columns, label, className) {
	const root = document.createElement("div");
	root.className = ["tmt-ui-list-table", className].filter(Boolean).join(" ");
	root.setAttribute("role", "table");
	root.setAttribute("aria-label", label);
	const header = document.createElement("div");
	header.className = "tmt-ui-list-columns";
	header.setAttribute("role", "row");
	for (const column of columns) {
		const cell = document.createElement("span");
		cell.setAttribute("role", "columnheader");
		if (column.hideBelow) cell.dataset.hideBelow = column.hideBelow;
		cell.textContent = column.label;
		header.append(cell);
	}
	const body = document.createElement("div");
	body.setAttribute("role", "rowgroup");
	root.append(header, body);
	return {
		root,
		body,
		row(cells, data = {}) {
			if (cells.length !== columns.length) throw new Error("A cell is required for every column.");
			const row = document.createElement("div");
			row.className = "tmt-ui-list-table-row";
			row.setAttribute("role", "row");
			for (const [name, value] of Object.entries(data)) if (value !== void 0) row.setAttribute(`data-${name}`, value);
			cells.forEach((content, index) => {
				const cell = document.createElement("div");
				cell.setAttribute("role", "cell");
				cell.dataset.column = columns[index].id;
				if (columns[index]?.hideBelow) cell.dataset.hideBelow = columns[index].hideBelow;
				cell.append(content);
				row.append(cell);
			});
			return row;
		}
	};
}
/** Framework-free subheader. The caller owns its actions and the top-bar title. */
function createPageSubheader(options) {
	const root = document.createElement("header");
	root.className = "tmt-ui-subheader";
	const text = document.createElement("div");
	text.className = "tmt-ui-subheader-text";
	const heading = document.createElement(`h${options.headingLevel ?? 2}`);
	heading.textContent = options.title;
	if (options.count !== void 0) {
		const count = document.createElement("span");
		count.className = "tmt-ui-shell-count";
		count.textContent = String(options.count);
		heading.append(" ", count);
	}
	text.append(heading);
	if (options.subtitle !== void 0) {
		const subtitle = document.createElement("p");
		subtitle.textContent = options.subtitle;
		text.append(subtitle);
	}
	root.append(text);
	if (options.actions) {
		const actions = document.createElement("div");
		actions.className = "tmt-ui-subheader-actions";
		actions.append(options.actions);
		root.append(actions);
	}
	return root;
}
new TextEncoder();
function requireValue(condition, label) {
	if (!condition) throw new Error(`Invalid ${label}.`);
}
var BASE64URL = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
/** Unpadded RFC 4648 base64url. */
function base64url(bytes) {
	requireValue(bytes instanceof Uint8Array, "base64url bytes");
	let text = "";
	for (let index = 0; index < bytes.length; index += 3) {
		const chunk = bytes[index] << 16 | (bytes[index + 1] ?? 0) << 8 | (bytes[index + 2] ?? 0);
		const symbols = Math.min(4, Math.ceil((bytes.length - index) * 8 / 6));
		for (let symbol = 0; symbol < symbols; symbol++) text += BASE64URL[chunk >> 18 - 6 * symbol & 63];
	}
	return text;
}
//#endregion
//#region src/operations.ts
/** Known refusal; send/operation return pre-effect codes as state instead. */
var RefusalError$1 = class extends Error {
	code;
	retryAfterMs;
	limit;
	settingsUrl;
	scope;
	constructor(code, retryAfterMs, limit, settingsUrl, scope) {
		super(`Remote operation refused: ${code}.`);
		this.code = code;
		this.retryAfterMs = retryAfterMs;
		this.limit = limit;
		this.settingsUrl = settingsUrl;
		this.scope = scope;
		this.name = "RefusalError";
	}
};
new TextEncoder();
new TextDecoder("utf-8", { fatal: true });
//#endregion
//#region src/firebase-bootstrap.ts
/** Hosted public configuration is a corruption-checked carrier, never an access grant. */
var encoder$1 = new TextEncoder();
var decoder$1 = new TextDecoder("utf-8", { fatal: true });
var LINK_BYTES = 8192;
function valid$1(condition) {
	if (!condition) throw new RefusalError$1("REMOTE_INPUT_INVALID");
}
function object(value, keys) {
	valid$1(typeof value === "object" && value !== null && !Array.isArray(value));
	const record = value;
	valid$1(Object.keys(record).sort().join(",") === [...keys].sort().join(","));
	return record;
}
function parsePublicWebConfig(value) {
	const r = object(value, [
		"apiKey",
		"appId",
		"authDomain",
		"projectId"
	]);
	valid$1(typeof r.projectId === "string" && /^[a-z][a-z0-9-]{4,28}[a-z0-9]$/.test(r.projectId));
	valid$1(r.authDomain === `${r.projectId}.firebaseapp.com`);
	for (const key of ["apiKey", "appId"]) valid$1(typeof r[key] === "string" && /^[\x20-\x7e]{1,256}$/.test(r[key]));
	return Object.freeze(r);
}
function canonical(value) {
	if (typeof value !== "object" || value === null) return JSON.stringify(value);
	const r = value;
	return `{${Object.keys(r).sort().map((k) => `${JSON.stringify(k)}:${canonical(r[k])}`).join(",")}}`;
}
async function checksum(value) {
	return Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", encoder$1.encode(canonical(value)))), (b) => b.toString(16).padStart(2, "0")).join("");
}
async function verifyHostedLink(link, origin) {
	try {
		valid$1(typeof link === "string" && encoder$1.encode(link).length <= LINK_BYTES);
		const url = new URL(link);
		valid$1(url.origin === origin && url.protocol === "https:" && url.pathname === "/pair" && url.search === "" && url.username === "" && url.password === "");
		const fragment = url.hash.slice(1);
		valid$1(/^[A-Za-z0-9_-]+$/.test(fragment));
		const bytes = Uint8Array.from(atob(fragment.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
		const text = decoder$1.decode(bytes);
		const raw = JSON.parse(text);
		const hasSite = typeof raw === "object" && raw !== null && Object.hasOwn(raw, "hostingSite");
		const r = object(raw, [
			"version",
			"kind",
			"publicWebConfig",
			"checksum",
			...hasSite ? ["hostingSite"] : []
		]);
		valid$1(r.version === 1 && r.kind === "firestore" && typeof r.checksum === "string" && /^[a-f0-9]{64}$/.test(r.checksum));
		const publicConfig = parsePublicWebConfig(r.publicWebConfig);
		let site = publicConfig.projectId;
		if (hasSite) {
			const selected = r.hostingSite;
			valid$1(typeof selected === "string" && /^[a-z0-9](?:[a-z0-9-]{0,28}[a-z0-9])?$/.test(selected) && selected !== publicConfig.projectId);
			site = selected;
		}
		valid$1(origin === `https://${site}.web.app`);
		valid$1(await checksum({
			kind: r.kind,
			publicWebConfig: publicConfig,
			version: r.version,
			...hasSite ? { hostingSite: site } : {}
		}) === r.checksum && text === canonical(r));
		valid$1(link === `${origin}/pair#${base64url(bytes)}`);
		return Object.freeze({
			origin,
			publicWebConfig: publicConfig,
			...hasSite ? { hostingSite: site } : {}
		});
	} catch {
		throw new RefusalError$1("REMOTE_INPUT_INVALID");
	}
}
//#endregion
//#region src/browser-store.ts
/** Private origin-local opener. Public registry APIs never expose the device record. */
function request(open) {
	return new Promise((resolve, reject) => {
		const pending = open();
		pending.onsuccess = () => resolve(pending.result);
		pending.onerror = () => reject(pending.error ?? /* @__PURE__ */ new Error("Device storage failed."));
	});
}
async function deviceObjects(mode) {
	const opening = indexedDB.open("tmt-remote", 1);
	opening.onupgradeneeded = () => opening.result.createObjectStore("device");
	return (await request(() => opening)).transaction("device", mode).objectStore("device");
}
var REGISTRY = "remotes-v1";
async function readRegistry() {
	const objects = await deviceObjects("readonly");
	try {
		return await request(() => objects.get(REGISTRY));
	} finally {
		objects.transaction.db.close();
	}
}
/** Compare and replace one public key atomically; no retry, clear or device access. */
async function writeRegistry(expected, text) {
	const objects = await deviceObjects("readwrite");
	const transaction = objects.transaction;
	await new Promise((resolve, reject) => {
		let conflict = false;
		transaction.oncomplete = () => resolve();
		transaction.onabort = transaction.onerror = () => reject(/* @__PURE__ */ new Error(conflict ? "Registry changed" : "Registry unavailable"));
		const read = objects.get(REGISTRY);
		read.onsuccess = () => {
			if (read.result !== expected) {
				conflict = true;
				transaction.abort();
			} else objects.put(text, REGISTRY);
		};
	}).finally(() => transaction.db.close());
}
//#endregion
//#region src/remote-registry.ts
/** Public references only; keys, pairings, credentials, drafts and intents never enter a catalog. */
/** One projection feeds the list and rail; this machine is first even without a saved alias. */
function registryView(catalog, observed) {
	const selected = catalog.entries.find((entry) => currentRemote(entry, observed.address, observed.pageUrl));
	const row = (entry, thisMachine) => ({
		id: entry?.name ?? "@this-machine",
		name: entry?.name ?? "This machine",
		url: entry?.url ?? currentDoor(observed.pageUrl),
		reference: entry,
		thisMachine,
		backend: entry?.kind === "firestore" ? "Firebase" : "Local",
		project: entry?.project ?? null,
		role: thisMachine ? observed.role : void 0,
		roleLabel: remoteRole(thisMachine ? observed.role : void 0),
		connection: remoteState(observed.state, thisMachine)
	});
	const current = row(selected ?? null, true);
	return {
		rows: [current, ...catalog.entries.filter((entry) => entry !== selected).map((entry) => row(entry, false))],
		current,
		address: observed.address
	};
}
var RemoteRegistryError = class extends Error {
	code;
	constructor(code, message) {
		super(message);
		this.code = code;
	}
};
var LIMIT = 8192;
var encoder = new TextEncoder();
var decoder = new TextDecoder("utf-8", { fatal: true });
var empty = {
	version: 1,
	entries: []
};
function valid(condition, message = "The remote name or link is invalid") {
	if (!condition) throw new RemoteRegistryError("REMOTE_SELECTOR_INVALID", message);
}
function fields(value, keys) {
	valid(typeof value === "object" && value !== null && !Array.isArray(value));
	const object = value;
	valid(Object.keys(object).sort().join(",") === [...keys].sort().join(","));
	return object;
}
function name(value) {
	valid(typeof value === "string" && /^[a-z0-9-]{1,63}$/.test(value) && value !== "local");
}
function target(entry) {
	return new URL(entry.url).href;
}
async function reference(alias, address) {
	name(alias);
	valid(typeof address === "string" && address.length <= LIMIT);
	let url;
	try {
		url = new URL(address);
	} catch {
		valid(false);
	}
	if (url.protocol === "https:" && url.hostname.endsWith(".web.app") && url.pathname === "/pair") {
		const { publicWebConfig: config } = await verifyHostedLink(address, url.origin).catch(() => {
			valid(false);
		});
		return {
			name: alias,
			kind: "firestore",
			url: address,
			project: config.projectId
		};
	}
	valid(address.length <= 2048 && /^https?:\/\/[A-Za-z0-9.:-]+\/[A-Za-z0-9/_-]*$/.test(address) && address.endsWith("/") && !url.search && !url.hash && !url.username && !url.password);
	return {
		name: alias,
		kind: "local",
		url: address,
		project: null
	};
}
async function parseCatalog(value) {
	const row = fields(value, ["version", "entries"]);
	valid(row.version === 1 && Array.isArray(row.entries));
	valid(row.entries.length <= 16, "This browser can store at most 16 remotes. Remove one before adding another.");
	const entries = [];
	for (const raw of row.entries) {
		const r = fields(raw, [
			"name",
			"kind",
			"url",
			"project"
		]);
		const entry = await reference(r.name, r.url);
		valid(entry.kind === r.kind && entry.project === r.project);
		valid(!entries.some((old) => old.name === entry.name || target(old) === target(entry)));
		entries.push(Object.freeze(entry));
	}
	const catalog = {
		version: 1,
		entries: Object.freeze(entries)
	};
	valid(encoder.encode(json(catalog)).length <= LIMIT, "The remote list is too large to store in this browser. Remove one before adding another.");
	return Object.freeze(catalog);
}
function json(catalog) {
	return JSON.stringify(catalog);
}
function decode(value) {
	valid(/^[A-Za-z0-9_-]+$/.test(value));
	try {
		const bytes = Uint8Array.from(atob(value.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
		valid(base64url(bytes) === value);
		return JSON.parse(decoder.decode(bytes));
	} catch {
		valid(false);
	}
}
function hint(value) {
	const r = fields(value, ["spaceId", "pageId"]);
	valid(typeof r.spaceId === "string" && /^[a-z2-7]{32}$/.test(r.spaceId));
	valid(typeof r.pageId === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(r.pageId));
	return {
		spaceId: r.spaceId,
		pageId: r.pageId
	};
}
async function parseHandoff(fragment) {
	valid(fragment.startsWith("#") && encoder.encode(fragment).length <= LIMIT);
	const params = new URLSearchParams(fragment.slice(1));
	const keys = [...params.keys()];
	valid(keys.length === new Set(keys).size && keys.every((key) => ["tmt-remotes", "tmt-page"].includes(key)));
	const raw = params.get("tmt-remotes");
	valid(raw !== null);
	const catalog = await parseCatalog(decode(raw));
	valid(base64url(encoder.encode(json(catalog))) === raw);
	const page = params.get("tmt-page");
	const checkedHint = page === null ? void 0 : hint(decode(page));
	if (checkedHint) valid(base64url(encoder.encode(JSON.stringify(checkedHint))) === page);
	return {
		catalog,
		...checkedHint ? { hint: checkedHint } : {}
	};
}
async function buildHandoff(destination, value, page) {
	const checked = await reference(destination.name, destination.url);
	valid(checked.kind === destination.kind && checked.project === destination.project);
	if (checked.kind === "firestore") return checked.url;
	const catalog = await parseCatalog(value);
	const params = new URLSearchParams({ "tmt-remotes": base64url(encoder.encode(json(catalog))) });
	if (page !== void 0) params.set("tmt-page", base64url(encoder.encode(JSON.stringify(hint(page)))));
	valid(encoder.encode(`#${params}`).length <= LIMIT, "The remote list is too large to send along when opening this remote. Remove an entry, then try again.");
	return `${new URL("/remote/overview", checked.url).href}#${params}`;
}
async function planImport(incoming, current) {
	const next = await parseCatalog(incoming), before = await parseCatalog(current);
	const used = new Set([...before.entries, ...next.entries].map((entry) => entry.name));
	return {
		incoming: next,
		current: before,
		rows: next.entries.map((entry) => {
			const same = before.entries.find((old) => target(old) === target(entry));
			if (same) return {
				entry,
				status: "duplicate",
				suggestedName: entry.name,
				existingName: same.name
			};
			if (!before.entries.some((old) => old.name === entry.name)) return {
				entry,
				status: "add",
				suggestedName: entry.name
			};
			let n = 2;
			let proposed;
			do {
				const suffix = `-${n++}`;
				proposed = entry.name.slice(0, 63 - suffix.length) + suffix;
			} while (used.has(proposed));
			used.add(proposed);
			return {
				entry,
				status: "collision",
				suggestedName: proposed
			};
		})
	};
}
async function mergeImport(plan, choices) {
	const checked = await planImport(plan.incoming, plan.current);
	valid(choices.length === checked.rows.length);
	const entries = [...checked.current.entries];
	for (const [index, row] of checked.rows.entries()) {
		const alias = choices[index];
		if (alias === null || row.status === "duplicate") continue;
		name(alias);
		if (entries.some((old) => old.name === alias)) throw new RemoteRegistryError("REMOTE_SELECTOR_CONFLICT", `You already have a remote named ${alias}`);
		entries.push({
			...row.entry,
			name: alias
		});
	}
	return parseCatalog({
		version: 1,
		entries
	});
}
async function snapshot() {
	try {
		const raw = await readRegistry();
		valid(raw === void 0 || typeof raw === "string" && raw.length <= LIMIT && encoder.encode(raw).length <= LIMIT);
		const catalog = raw === void 0 ? empty : await parseCatalog(JSON.parse(raw));
		valid(raw === void 0 || raw === json(catalog));
		return {
			raw,
			catalog
		};
	} catch {
		throw new RemoteRegistryError("REMOTE_SELECTOR_UNAVAILABLE", "The remotes could not be read");
	}
}
async function commit(raw, catalog) {
	try {
		await writeRegistry(raw, json(catalog));
	} catch (error) {
		throw new RemoteRegistryError(error instanceof Error && error.message === "Registry changed" ? "REMOTE_SELECTOR_CONFLICT" : "REMOTE_SELECTOR_UNAVAILABLE", "The remotes could not be saved. Check the list again.");
	}
}
var remoteRegistry = {
	parseHandoff,
	async list() {
		return (await snapshot()).catalog;
	},
	async planImport(incoming) {
		return planImport(incoming, (await snapshot()).catalog);
	},
	async confirmImport(plan, choices) {
		const next = await mergeImport(plan, choices), latest = await snapshot();
		if (json(latest.catalog) !== json(plan.current)) throw new RemoteRegistryError("REMOTE_SELECTOR_CONFLICT", "The remotes changed. Check the list again.");
		await commit(latest.raw, next);
	},
	async add(alias, url) {
		const entry = await reference(alias, url), latest = await snapshot();
		const plan = await planImport({
			version: 1,
			entries: [entry]
		}, latest.catalog);
		valid(plan.rows[0].status !== "duplicate", `Already added as ${plan.rows[0].existingName}`);
		await remoteRegistry.confirmImport(plan, [alias]);
	},
	async rename(previous, alias) {
		name(alias);
		const latest = await snapshot();
		valid(latest.catalog.entries.some((entry) => entry.name === previous));
		valid(!latest.catalog.entries.some((entry) => entry.name === alias && entry.name !== previous), `You already have a remote named ${alias}`);
		const next = await parseCatalog({
			version: 1,
			entries: latest.catalog.entries.map((entry) => entry.name === previous ? {
				...entry,
				name: alias
			} : entry)
		});
		await commit(latest.raw, next);
	},
	async remove(alias) {
		const latest = await snapshot();
		valid(latest.catalog.entries.some((entry) => entry.name === alias));
		await commit(latest.raw, {
			version: 1,
			entries: latest.catalog.entries.filter((entry) => entry.name !== alias)
		});
	},
	async navigation(entry, page) {
		return buildHandoff(entry, await remoteRegistry.list(), page);
	}
};
//#endregion
//#region src/remote-registry-page.ts
/** Remote pages' public-reference consumer. It never probes a foreign origin or shares a key. */
function node$2(tag, text = "") {
	const result = document.createElement(tag);
	result.textContent = text;
	return result;
}
function paintConnection(state, connection) {
	state.dataset.tone = connection.text === "Connected" ? "working" : connection.text === "Offline" ? "blocked" : "neutral";
	const mark = node$2("span");
	mark.className = browserUiClasses.stateMark;
	mark.dataset.shape = "dot";
	mark.setAttribute("aria-hidden", "true");
	const label = node$2("span", connection.text);
	label.className = browserUiClasses.chipLabel;
	state.replaceChildren(mark, label);
}
function action(text, run, variant) {
	const button = node$2("button");
	const label = node$2("span", text);
	label.className = browserUiClasses.actionLabel;
	button.append(label);
	if (variant) button.dataset.variant = variant;
	button.type = "button";
	button.className = "tmt-ui-action";
	button.addEventListener("click", run);
	return button;
}
var fieldCount = 0;
function field(label, value) {
	const wrapper = node$2("div");
	wrapper.className = browserUiClasses.field;
	const input = node$2("input");
	input.id = `remote-field-${++fieldCount}`;
	input.className = browserUiClasses.fieldControl;
	input.value = value;
	const text = node$2("label", label);
	text.className = browserUiClasses.fieldLabel;
	text.htmlFor = input.id;
	wrapper.append(text, input);
	return {
		label: wrapper,
		input
	};
}
function kindLabel(entry) {
	return entry.kind === "local" ? "Local" : "Firebase";
}
function remoteRegistryPage(initialBlocked = () => "", shell) {
	let heldBy = initialBlocked;
	const blocked = () => heldBy();
	const root = document.getElementById("remotes");
	if (!root) return Object.assign(() => {}, {
		blockedBy: () => {},
		setSummary: () => {},
		watchSummary: () => () => {},
		renameCurrent: () => {},
		watch: () => () => {}
	});
	const listeners = /* @__PURE__ */ new Set();
	const summaryListeners = /* @__PURE__ */ new Set();
	const fragment = location.hash;
	if (fragment) history.replaceState(null, "", location.pathname + location.search);
	const oldList = document.getElementById("remote-list");
	const table = createListTable([
		{
			id: "icon",
			label: "",
			hideBelow: "phone"
		},
		{
			id: "name",
			label: "Name"
		},
		{
			id: "connection",
			label: "Connection"
		},
		{
			id: "role",
			label: "Your role"
		},
		{
			id: "members",
			label: "Members",
			hideBelow: "phone"
		},
		{
			id: "active",
			label: "Last active",
			hideBelow: "phone"
		},
		{
			id: "actions",
			label: ""
		}
	], "My Remotes", "remote-registry-table remote-phone-rows");
	const list = table.body;
	list.id = "remote-list";
	oldList.replaceWith(table.root);
	const footer = node$2("p");
	footer.className = "remote-list-footer";
	table.root.after(footer);
	const head = document.getElementById("remote-head");
	const actions = head.querySelector(".remote-head-actions");
	const primary = actions.querySelector("a[data-variant=primary]");
	if (primary) {
		primary.prepend(createIcon("plus"));
		actions.prepend(primary);
	}
	const subheader = createPageSubheader({
		title: "My Remotes",
		subtitle: "Names and addresses stay in this browser.",
		actions
	});
	head.replaceChildren(...subheader.childNodes);
	head.className = subheader.className;
	const subtitle = head.querySelector(".tmt-ui-subheader-text p");
	const status = document.getElementById("remote-status");
	const joinHost = document.getElementById("remote-join");
	const joinMenu = mountBrowserMenu(joinHost, {
		label: "Join a Remote",
		triggerText: "Join",
		items: [{
			key: "invite",
			label: "Join with an invitation"
		}, {
			key: "address",
			label: "Remember a Remote",
			separated: true
		}],
		onSelect: (key) => {
			if (key === "address") openAdd();
			else (shell ?? void 0)?.navigate(new URL(remotePath("join"), location.href).href);
		}
	});
	const join = joinHost.querySelector("button");
	let rowMenus = [];
	(shell ?? void 0)?.signal.addEventListener("abort", () => {
		joinMenu.dispose();
		rowMenus.forEach((item) => item.dispose());
	}, { once: true });
	let observed = "Checking";
	let observedRole;
	let summary;
	let address = document.documentElement.dataset.tmtPage === "settings" ? new URL("./", location.href).href : void 0;
	let currentDialog;
	(shell ?? void 0)?.signal.addEventListener("abort", () => {
		const leaving = currentDialog;
		currentDialog = void 0;
		leaving?.close();
		leaving?.remove();
	}, { once: true });
	let catalog = {
		version: 1,
		entries: []
	};
	const menu = remoteMenu({
		refresh: () => render(),
		recover: (action) => {
			const home = new URL(remotePath("overview"), location.href).href;
			if (action === "Retry") {
				const retry = ["check", "refresh"].map((id) => document.getElementById(id)).find((button) => button && button.getClientRects().length > 0);
				if (retry && !retry.disabled) retry.click();
				else (shell ?? void 0)?.navigate(home);
			} else {
				if (location.pathname !== remotePath("overview")) (shell ?? void 0)?.navigate(home);
				if (location.pathname !== remotePath("overview")) return;
				const pair = document.getElementById("copy-guide-pair");
				pair?.scrollIntoView();
				pair?.focus();
			}
		}
	}, shell);
	const showError = (error, slot) => {
		slot.textContent = error instanceof RemoteRegistryError ? error.message : "The remotes could not be read";
	};
	const dialog = (title, opener) => {
		currentDialog?.close();
		const box = node$2("dialog");
		box.className = browserUiClasses.dialog;
		const header = node$2("header");
		header.className = browserUiClasses.dialogHeader;
		const heading = node$2("h2", title);
		heading.id = "remote-dialog-heading";
		header.append(heading);
		box.setAttribute("aria-labelledby", heading.id);
		const content = node$2("div");
		content.className = browserUiClasses.dialogBody;
		const body = node$2("div");
		const message = node$2("p");
		message.setAttribute("role", "status");
		content.append(body, message);
		const buttons = node$2("footer");
		buttons.className = browserUiClasses.dialogActions;
		box.append(header, content, buttons);
		document.body.append(box);
		const end = () => box.close();
		box.addEventListener("close", () => {
			box.remove();
			if (currentDialog === box) {
				currentDialog = void 0;
				(opener.isConnected ? opener : join).focus();
			}
		}, { once: true });
		currentDialog = box;
		return {
			box,
			body,
			buttons,
			message,
			end
		};
	};
	const save = async (run, view) => {
		const buttons = [...view.box.querySelectorAll("button")];
		buttons.forEach((b) => {
			b.disabled = true;
		});
		try {
			await run();
			await render();
			view.end();
			return true;
		} catch (error) {
			showError(error, view.message);
			return false;
		} finally {
			buttons.forEach((b) => {
				b.disabled = false;
			});
		}
	};
	const importView = (plan) => {
		const view = dialog("Add these remotes to this browser?", join);
		view.body.append(node$2("p", "Only names and addresses are copied. No keys, pairings or drafts."));
		const choices = [];
		for (const row of plan.rows) {
			const item = node$2("section");
			item.className = browserUiClasses.section;
			const name = node$2("h3", row.entry.name);
			name.className = browserUiClasses.sectionHeading;
			name.dataset.variant = "title";
			item.append(name, node$2("p", `${kindLabel(row.entry)} · ${row.entry.url}`));
			if (row.status === "duplicate") {
				item.append(node$2("p", `Already added as ${row.existingName}`));
				choices.push(() => null);
			} else if (row.status === "collision") {
				item.append(node$2("p", `You already have a remote named ${row.entry.name}`));
				const alias = field("Remote name", row.suggestedName);
				const skip = node$2("input");
				skip.type = "checkbox";
				skip.className = browserUiClasses.checkbox;
				const skipLabel = node$2("label", "Skip");
				skipLabel.className = browserUiClasses.checkboxLabel;
				skipLabel.prepend(skip);
				item.append(alias.label, skipLabel);
				choices.push(() => skip.checked ? null : alias.input.value);
			} else choices.push(() => row.entry.name);
			view.body.append(item);
		}
		const submit = action("Add", () => {
			save(() => remoteRegistry.confirmImport(plan, choices.map((choice) => choice())), view);
		});
		const update = () => {
			submit.disabled = choices.every((choice) => choice() === null);
			view.message.textContent = submit.disabled ? "Nothing new to add." : "";
		};
		view.body.addEventListener("change", update);
		update();
		view.buttons.append(submit, action("Cancel", view.end));
		view.box.showModal();
	};
	const confirmAdd = (entry, opener) => {
		const view = dialog(entry.kind === "firestore" ? `Add Firebase remote ${entry.project}?` : `Add remote ${entry.name}?`, opener);
		view.body.append(node$2("p", entry.kind === "firestore" ? new URL(entry.url).host : entry.url));
		view.buttons.append(action("Add", () => {
			save(() => remoteRegistry.add(entry.name, entry.url), view);
		}), action("Cancel", view.end));
		view.box.showModal();
	};
	const rename = (entry, opener) => {
		const view = dialog(`Rename ${entry.name}`, opener);
		const alias = field("Remote name", entry.name);
		view.body.append(alias.label);
		const submit = action("Save", () => {
			save(() => remoteRegistry.rename(entry.name, alias.input.value), view);
		});
		submit.disabled = true;
		alias.input.addEventListener("input", () => {
			submit.disabled = alias.input.value === entry.name;
		});
		view.buttons.append(submit, action("Cancel", view.end));
		view.box.showModal();
	};
	const remove = (entry, opener) => {
		const view = dialog(`Forget ${entry.name} in this browser?`, opener);
		view.body.append(node$2("p", "This only removes the entry here. It does not sign out or revoke access."));
		view.buttons.append(action("Forget", () => {
			save(() => remoteRegistry.remove(entry.name), view).then((saved) => {
				if (!saved) return;
				status.replaceChildren(node$2("span", `Forgot ${entry.name} in this browser. Access is unchanged. `), action("Undo", () => {
					remoteRegistry.add(entry.name, entry.url).then(async () => {
						await render();
						status.textContent = `Restored ${entry.name}.`;
					}).catch((error) => showError(error, status));
				}));
			});
		}, "destructive"), action("Cancel", view.end));
		view.box.showModal();
	};
	const paint = () => {
		const projection = registryView(catalog, {
			pageUrl: location.href,
			state: observed,
			address,
			role: observedRole
		});
		const visibleSummary = observed === "Connected" && observedRole === "owner" ? summary : void 0;
		summaryListeners.forEach((listener) => listener(visibleSummary));
		if (subtitle) subtitle.textContent = `${projection.rows.length} ${projection.rows.length === 1 ? "remote" : "remotes"} · Names and addresses stay in this browser.`;
		footer.replaceChildren(node$2("span", `${projection.rows.length} ${projection.rows.length === 1 ? "remote" : "remotes"} in this view`), node$2("span", "Saved in this browser · Forget leaves access unchanged"));
		listeners.forEach((listener) => listener(projection));
		const existing = [...list.children];
		if (existing.length === projection.rows.length && existing.every((row, index) => {
			const entry = projection.rows[index];
			return row.dataset.remote === entry.id && row.dataset.url === entry.url && row.dataset.thisMachine === String(entry.thisMachine);
		})) {
			for (const [index, row] of existing.entries()) {
				const entry = projection.rows[index];
				const role = row.querySelector("[data-role]");
				role.querySelector(".tmt-ui-chip-label").textContent = entry.roleLabel;
				role.dataset.roleTone = entry.role === "owner" ? "selected" : entry.role === void 0 ? "muted" : "neutral";
				paintConnection(row.querySelector(".tmt-ui-list-state"), entry.connection);
				const countCell = row.querySelector("[data-summary=members]");
				const activeCell = row.querySelector("[data-summary=active]");
				countCell.textContent = entry.thisMachine && observedRole === "owner" && summary?.memberCount != null ? String(summary.memberCount) : "";
				activeCell.textContent = entry.thisMachine && observedRole === "owner" && summary?.lastActiveAtMs != null ? new Date(summary.lastActiveAtMs).toLocaleString() : "";
				const open = row.querySelector("[data-action=\"open\"]");
				open.setAttribute("aria-disabled", String(!entry.thisMachine && !!blocked()));
				if (!entry.thisMachine && blocked()) open.removeAttribute("href");
				else open.href = entry.thisMachine ? new URL(remotePath("overview"), location.href).href : entry.url;
				open.title = !entry.thisMachine ? blocked() : "";
			}
			menu(projection);
			return;
		}
		rowMenus.forEach((item) => item.dispose());
		rowMenus = [];
		list.replaceChildren(...projection.rows.map((entry) => {
			const leading = node$2("div");
			leading.className = browserUiClasses.listLeading;
			leading.append(createIcon(entry.backend === "Local" ? "terminal" : "box"));
			const title = node$2("div");
			title.className = browserUiClasses.listTitle;
			title.dataset.phone = "name";
			const label = node$2("span");
			label.className = `${browserUiClasses.listLabel} remote-label`;
			const open = node$2("a", entry.name);
			open.href = entry.thisMachine ? new URL(remotePath("overview"), location.href).href : entry.url;
			open.dataset.action = "open";
			open.setAttribute("role", "link");
			open.setAttribute("aria-label", `Open ${entry.name}`);
			open.setAttribute("aria-disabled", String(!entry.thisMachine && !!blocked()));
			if (!entry.thisMachine && blocked()) open.removeAttribute("href");
			open.title = !entry.thisMachine ? blocked() : "";
			open.addEventListener("click", (event) => {
				event.preventDefault();
				if (entry.thisMachine) {
					(shell ?? void 0)?.navigate(new URL(remotePath("overview"), location.href).href);
					return;
				}
				(async () => {
					try {
						if (!blocked() && entry.reference) location.assign(await remoteRegistry.navigation(entry.reference));
					} catch (error) {
						showError(error, status);
					}
				})();
			});
			label.append(open);
			const chip = (text) => {
				const box = node$2("span");
				box.className = browserUiClasses.chip;
				const value = node$2("span", text);
				value.className = browserUiClasses.chipLabel;
				box.append(value);
				return box;
			};
			if (entry.thisMachine && entry.name !== "This machine") {
				const current = chip("This machine");
				current.className += " remote-role";
				label.append(current);
			}
			const role = chip(entry.roleLabel);
			role.className += " remote-role";
			role.dataset.role = "";
			role.dataset.roleTone = entry.role === "owner" ? "selected" : entry.role === void 0 ? "muted" : "neutral";
			const description = node$2("span", `${entry.backend}${entry.project ? ` · ${entry.project}` : ""} · ${entry.url}`);
			description.className = browserUiClasses.listDescription;
			title.append(label, description);
			const state = node$2("div");
			state.className = `${browserUiClasses.listState} ${browserUiClasses.chip} remote-stage`;
			paintConnection(state, entry.connection);
			const actions = node$2("div");
			actions.className = browserUiClasses.listActions;
			actions.dataset.phone = "menu";
			if (entry.reference) {
				const reference = entry.reference;
				const destructive = node$2("span", "Forget");
				destructive.className = "remote-forget";
				const rowMenu = mountBrowserMenu(actions, {
					label: `Actions for ${entry.name}`,
					items: [{
						key: "rename",
						label: "Rename"
					}, {
						key: "forget",
						label: "Forget",
						content: destructive,
						separated: true
					}],
					onSelect: (key) => {
						const opener = actions.querySelector("button");
						if (key === "rename") rename(reference, opener);
						else remove(reference, opener);
					}
				});
				rowMenus.push(rowMenu);
			}
			const memberCount = node$2("span", entry.thisMachine && observedRole === "owner" && summary?.memberCount != null ? String(summary.memberCount) : "");
			memberCount.dataset.summary = "members";
			const lastActive = node$2("span", entry.thisMachine && observedRole === "owner" && summary?.lastActiveAtMs != null ? new Date(summary.lastActiveAtMs).toLocaleString() : "");
			lastActive.dataset.summary = "active";
			const row = table.row([
				leading,
				title,
				state,
				role,
				memberCount,
				lastActive,
				actions
			]);
			row.dataset.remote = entry.id;
			row.dataset.thisMachine = String(entry.thisMachine);
			row.dataset.url = entry.url;
			row.dataset.kind = entry.reference?.kind ?? "local";
			return row;
		}));
		menu(projection);
	};
	const render = async () => {
		try {
			catalog = await remoteRegistry.list();
			paint();
		} catch (error) {
			showError(error, status);
		}
	};
	function openAdd(address = "", opener = join) {
		const view = dialog("Add remote", opener);
		const alias = field("Remote name", "");
		const url = field("Remote address or link", address);
		view.body.append(alias.label, url.label);
		view.buttons.append(action("Continue", () => {
			reference(alias.input.value, url.input.value).then((entry) => {
				if (currentDialog !== view.box || !view.box.open || !view.box.isConnected) return;
				view.end();
				confirmAdd(entry, opener);
			}).catch((error) => showError(error, view.message));
		}), action("Cancel", view.end));
		view.box.showModal();
	}
	render().then(() => {
		if (fragment === "#add-remote") openAdd();
		else if (fragment === "#remotes") {
			root.scrollIntoView();
			join.focus({ preventScroll: true });
		}
	});
	const receive = (value) => {
		remoteRegistry.parseHandoff(value).then(async (packet) => {
			const plan = await remoteRegistry.planImport(packet.catalog);
			importView(plan);
		}).catch((error) => showError(error, status));
	};
	if (fragment && !["#remotes", "#add-remote"].includes(fragment)) receive(fragment);
	window.addEventListener("hashchange", () => {
		const value = location.hash;
		if (location.pathname === remotePath("join")) return;
		if (!value || ["#remotes", "#add-remote"].includes(value)) return;
		history.replaceState(null, "", location.pathname + location.search);
		receive(value);
	}, { signal: (shell ?? void 0)?.signal });
	const observe = (state, checkedAddress, role) => {
		observed = state;
		if (state !== "Connected" || role === null) {
			observedRole = void 0;
			summary = void 0;
		} else if (role !== void 0) observedRole = role;
		if (checkedAddress) address = checkedAddress;
		paint();
	};
	return Object.assign(observe, {
		setSummary: (value) => {
			summary = observed === "Connected" ? value : void 0;
			paint();
		},
		watchSummary: (listener) => {
			summaryListeners.add(listener);
			listener(observed === "Connected" && observedRole === "owner" ? summary : void 0);
			return () => summaryListeners.delete(listener);
		},
		renameCurrent: (opener) => {
			const current = registryView(catalog, {
				pageUrl: location.href,
				state: observed,
				address,
				role: observedRole
			}).current;
			if (current.reference) rename(current.reference, opener);
			else openAdd(current.url, opener);
		},
		watch: (listener) => {
			listeners.add(listener);
			listener(registryView(catalog, {
				pageUrl: location.href,
				state: observed,
				address,
				role: observedRole
			}));
			return () => listeners.delete(listener);
		},
		blockedBy: (reason) => {
			(shell ?? void 0)?.holdNavigation?.(reason);
			heldBy = reason;
		}
	});
}
var service_commands_default = {
	status: "tmt remote status --layers",
	preview: "tmt remote deploy firestore --project='<project-id>' --region='<region>' --defaults --json",
	deploy: "tmt remote deploy firestore --project='<project-id>' --region='<region>' --defaults",
	nodeVersion: "node --version",
	firebaseVersion: "firebase --version",
	firebaseInstall: "npm install -g firebase-tools@15.29.0",
	firebaseLogin: "firebase login",
	join: "tmt remote join <code>"
};
//#endregion
//#region src/service-commands.ts
/** Copy-only terminal references. Native grammar tests admit the same product catalog. */
var serviceCommands = Object.freeze(service_commands_default);
/** Accept concrete native input spellings only; no shell syntax can enter a printed command. */
function deploymentCommands(project, region) {
	if (!/^[a-z][a-z0-9-]{4,28}[a-z0-9]$/.test(project)) throw new Error("Enter a Firebase project ID: 6–30 lowercase letters, digits or hyphens.");
	if (!/^[a-z0-9-]{1,32}$/.test(region)) throw new Error("Enter the database region from your Firebase project.");
	const fill = (template) => template.replace("<project-id>", project).replace("<region>", region);
	return {
		preview: fill(service_commands_default.preview),
		deploy: fill(service_commands_default.deploy)
	};
}
//#endregion
//#region src/firestore.ts
var firestoreTitles = {
	sharing: "Page sharing",
	operations: "Colab sync",
	attachments: "File storage (paid add-on)"
};
var firestoreItems = {
	project: "Project",
	"sign-in": "Sign-in",
	rules: "Rules",
	indexes: "Firestore indexes",
	"plan-tier": "Firebase plan",
	quota: "Free-plan quota",
	support: "Availability in this release"
};
var firestoreReasons = {
	project: {
		"not-configured": ["No Firebase project is set up.", true],
		"access-lost": ["tmt can no longer reach the Firebase project.", true],
		"not-checked": ["The Firebase project has not been checked.", false]
	},
	"sign-in": {
		"provider-disabled": ["Sign-in is not enabled for the project.", true],
		"permission-missing": ["This account isn't allowed to turn on sign-in for the project.", false],
		"not-checked": ["Sign-in has not been checked.", false]
	},
	rules: {
		"not-deployed": ["Firestore rules are not deployed.", true],
		"out-of-date": ["Firestore rules are out of date.", true],
		partial: ["Firestore rules are only partly deployed.", true],
		"not-checked": ["Firestore rules have not been checked.", false]
	},
	indexes: {
		building: [
			"Firestore indexes are still building.",
			true,
			"Wait a few minutes, then run tmt remote deploy firestore."
		],
		"not-ready": [
			"Firestore indexes have not been verified as ready.",
			true,
			"Run tmt remote deploy firestore."
		],
		"not-checked": [
			"Firestore index readiness has not been checked.",
			true,
			"Run tmt remote deploy firestore."
		]
	},
	"plan-tier": {
		"paid-plan-required": ["This needs a paid Firebase plan.", false],
		"not-checked": ["The Firebase plan has not been checked.", false]
	},
	quota: {
		"headroom-low": ["Today's free Firebase quota is almost used up.", false],
		exhausted: ["Today's free Firebase quota is used up.", false],
		"not-checked": ["The free Firebase quota has not been checked.", false]
	},
	support: { "not-implemented": ["This isn't available in this release yet.", false] }
};
//#endregion
//#region src/command-copy.ts
/** A terminal reference with best-effort Copy and inline status; it never executes the command. */
function commandCopy(text) {
	const command = document.createElement("div");
	command.className = browserUiClasses.command;
	const code = document.createElement("code");
	code.className = browserUiClasses.commandText;
	code.textContent = text;
	const copy = document.createElement("button");
	copy.className = browserUiClasses.commandCopy;
	copy.type = "button";
	copy.setAttribute("aria-label", `Copy ${text}`);
	copy.append(createIcon("copy"), "Copy");
	const feedback = document.createElement("span");
	feedback.className = browserUiClasses.commandFeedback;
	feedback.setAttribute("role", "status");
	copy.addEventListener("click", async () => {
		try {
			await navigator.clipboard.writeText(text);
			feedback.textContent = "Copied.";
		} catch {
			feedback.textContent = "Copy failed. Select the command and copy it manually.";
		}
	});
	command.append(code, copy, feedback);
	return command;
}
//#endregion
//#region src/firestore-page.ts
/** Read-only settings presentation; all evidence was validated on the signed SDK lane. */
var states = {
	enabled: [
		"✓",
		"Enabled",
		"working"
	],
	building: [
		"◌",
		"Building",
		"waiting"
	],
	"not-enabled": [
		"—",
		"Not enabled",
		"review"
	],
	unknown: [
		"○",
		"Not checked",
		"waiting"
	]
};
function node$1(tag, text = "") {
	const element = document.createElement(tag);
	element.textContent = text;
	return element;
}
function mark(state) {
	const [icon, text, tone] = states[state];
	const label = node$1("span");
	label.dataset.tone = tone;
	label.className = browserUiClasses.listState;
	const symbol = node$1("span", icon);
	symbol.setAttribute("aria-hidden", "true");
	label.append(symbol, document.createTextNode(` ${text}`));
	return label;
}
function allowance(amount, bytes, period) {
	let written = amount.toLocaleString("en-US");
	if (bytes) {
		const matched = [
			[1024 ** 3, "GiB"],
			[1024 ** 2, "MiB"],
			[1024, "KiB"]
		].find(([size]) => amount % size === 0);
		written = matched ? `${(amount / matched[0]).toLocaleString("en-US")} ${matched[1]}` : `${written} bytes`;
	}
	return `${written}${period ? ` per ${period}` : ""}`;
}
function link(text, href) {
	const anchor = node$1("a", text);
	anchor.href = href;
	anchor.target = "_blank";
	anchor.rel = "noopener noreferrer";
	return anchor;
}
function checks() {
	const section = node$1("section");
	section.className = "firestore-layer";
	section.setAttribute("aria-label", "Checks before an update");
	section.append(node$1("h3", "Before an update"));
	const rows = [{
		id: "version",
		title: "Deployed version and required version",
		detail: "Version comparison is not available here. Check this Remote’s setup in this machine’s terminal before upgrading.",
		command: serviceCommands.status
	}, {
		id: "changes",
		title: "What will change and account permissions",
		detail: "No deployment plan or account permissions have been checked here. Use the Firebase account that manages the project to preview both in this machine’s terminal.",
		command: serviceCommands.preview
	}];
	for (const row of rows) {
		const item = node$1("section");
		item.dataset.serviceCheck = row.id;
		const title = node$1("h4", `${row.title}: `);
		title.append(mark("unknown"));
		item.append(title, node$1("p", row.detail), commandCopy(row.command));
		section.append(item);
	}
	section.append(node$1("p", "Replace <project-id> and <region> with your Firebase project and its region. The preview command does not authorize deployment."));
	return section;
}
function guidance() {
	const section = node$1("section");
	section.className = "firestore-layer";
	section.append(node$1("h3", "Where to check and make changes"));
	const steps = node$1("ol");
	const consoleStep = node$1("li", "In the provider console, choose your Firebase project. Review its sign-in, database rules and usage. ");
	consoleStep.append(link("Open Firebase console", "https://console.firebase.google.com/"));
	const permissionStep = node$1("li", "In the provider console, open Project settings → Users and permissions. Ask the project administrator for missing access. ");
	permissionStep.append(link("Firebase roles and permissions", "https://firebase.google.com/docs/projects/iam/roles"));
	steps.append(node$1("li", "In this machine’s terminal, install the Firebase CLI and sign in with the account that manages the project."), consoleStep, node$1("li", "In this machine’s terminal, copy the preview command above to review changes and check permissions. Review and authorize any deployment there."), permissionStep, node$1("li", "After you act in the terminal or provider console, return here and use Refresh values. That rechecks this machine’s saved setup facts; it does not run a live provider check."));
	section.append(steps);
	return section;
}
function paidStorage() {
	const paid = node$1("p", "Paid Firebase Blaze usage and separate consent are required when file storage becomes available. ");
	paid.append(link("Firebase pricing plans", "https://firebase.google.com/docs/projects/billing/firebase-pricing-plans"));
	return paid;
}
function futureServices(includeStorage = false) {
	const section = node$1("section");
	section.className = "firestore-layer";
	section.setAttribute("aria-label", "Other services");
	section.append(node$1("h3", "Other services"));
	if (includeStorage) section.append(node$1("p", "File storage (paid add-on) — Not available in this release."), paidStorage());
	section.append(node$1("p", "Cloudflare — Coming soon."));
	return section;
}
var painted = /* @__PURE__ */ new WeakMap();
function renderFirestore(target, access, view, options = {}) {
	const projection = JSON.stringify([
		access,
		view,
		options.commands
	]);
	if (painted.get(target) === projection) return;
	painted.set(target, projection);
	const announcement = options.announcement ?? document.getElementById("firestore-announcement");
	const summary = access === "checking" ? "" : access !== "confirmed" || !view ? "Service status could not be confirmed." : !view.firestoreLayers.length ? "Firestore is not configured." : view.firestoreLayers.map((layer) => `${firestoreTitles[layer.layer]}: ${layer.prerequisites.some((item) => item.item === "support" && item.reason === "not-implemented") ? "Not available in this release" : states[layer.state][1]}.`).join(" ");
	if (summary && announcement.textContent !== summary) announcement.textContent = summary;
	target.replaceChildren();
	if (access !== "confirmed" || !view) {
		target.append(node$1("p", access === "checking" ? "Checking service status…" : "Service status could not be confirmed."));
		if (access !== "checking" && options.commands !== false) target.append(checks(), guidance(), futureServices(true));
		target.dataset.tone = access === "checking" ? "waiting" : "blocked";
		return;
	}
	target.dataset.tone = "review";
	target.append(node$1("p", view.firestoreLayers.length ? "Cloud service: Firebase (Firestore)." : "Firestore is not configured."));
	target.append(node$1("p", "These setup facts come from this machine. They are not a live check of your Firebase project."));
	for (const layer of view.firestoreLayers) {
		const group = node$1("section");
		group.className = "firestore-layer";
		group.dataset[options.prefix ? `${options.prefix}Layer` : "layer"] = layer.layer;
		const title = firestoreTitles[layer.layer];
		if (layer.prerequisites.some((p) => p.item === "support" && p.reason === "not-implemented")) {
			const line = node$1("h3", `${title} — Not available in this release.`);
			group.append(line);
			if (layer.layer === "attachments") group.append(paidStorage());
		} else {
			const heading = node$1("h3", `${title} `);
			heading.append(mark(layer.state));
			group.append(heading);
			const list = node$1("ul");
			for (const item of layer.prerequisites) {
				const row = node$1("li");
				row.dataset[options.prefix ? `${options.prefix}Item` : "item"] = item.item;
				const displayState = item.item === "indexes" && item.state !== "enabled" ? item.reason === "building" ? "building" : "unknown" : item.state;
				row.append(node$1("span", `${firestoreItems[item.item]}: `), mark(displayState));
				if (item.reason) row.append(node$1("p", firestoreReasons[item.item][item.reason][0]));
				if (item.next && options.commands !== false) {
					const nextLine = firestoreReasons[item.item][item.reason][2];
					const hint = node$1("p", nextLine ?? "Run this in this machine’s terminal:");
					row.append(hint, commandCopy(item.next));
				}
				list.append(row);
			}
			group.append(list);
			if (layer.layer === "sharing" && layer.state === "enabled") group.append(node$1("p", "Each client stops itself before the free-plan limits. TMT does not check the project's remaining quota, so Firebase may still refuse requests."));
		}
		target.append(group);
	}
	if (options.commands !== false) target.append(checks(), guidance());
	target.append(futureServices(!view.firestoreLayers.some((layer) => layer.layer === "attachments")));
	const details = node$1("details");
	details.id = `${options.prefix ? `${options.prefix}-` : ""}firestore-budget`;
	details.className = browserUiClasses.disclosure;
	details.append(node$1("summary", "Firestore free-plan limits"));
	const published = view.firestoreBudget;
	details.append(node$1("h3", `Published limits as of ${published.readOn}`));
	const table = node$1("table");
	table.className = browserUiClasses.table;
	const head = node$1("tr");
	for (const text of ["Limit", "Published allowance"]) {
		const cell = node$1("th", text);
		cell.scope = "col";
		head.append(cell);
	}
	const thead = node$1("thead");
	thead.append(head);
	table.append(thead);
	const body = node$1("tbody");
	for (const [key, label, per, bytes] of budget.publishedLimitRows) {
		const row = node$1("tr");
		const title = node$1("th", label);
		title.scope = "row";
		row.append(title, node$1("td", allowance(published.limits[key], bytes, per)));
		body.append(row);
	}
	table.append(body);
	details.append(table);
	details.append(node$1("p", "Google may change these limits. They are shared by the whole Firebase project. Daily limits reset around midnight Pacific time."));
	details.append(node$1("p", "Remote can't see your actual usage; check it in the Firebase console."));
	details.append(node$1("p", `Each device warns at ${published.guard.warnPercent}% and stops at ${published.guard.refusePercent}% of its share of the daily limits.`));
	target.append(details);
}
//#endregion
//#region src/create-page.ts
/** A terminal/provider-console guide over the retained management observation, never a deploy runner. */
function node(tag, text = "") {
	const element = document.createElement(tag);
	element.textContent = text;
	return element;
}
function external(text, href) {
	const link = node("a", text);
	link.href = href;
	link.target = "_blank";
	link.rel = "noopener noreferrer";
	return link;
}
function createPage(target, options) {
	const { signal } = options;
	let type = "local";
	let step = 1;
	const visited = /* @__PURE__ */ new Set();
	let rechecking = false;
	const buttons = [];
	const panels = [];
	const titles = [
		"Type",
		"Prepare",
		"Details",
		"Review",
		"Check result"
	];
	const steps = node("ol");
	steps.className = "create-steps";
	steps.setAttribute("aria-label", "Create guide progress");
	steps.setAttribute("aria-description", "Done marks visited guide steps, not checked prerequisites or deployment completion.");
	const content = node("div");
	const reference = (command) => {
		const block = commandCopy(command);
		block.querySelector("code").replaceChildren(...command.split(/( +)/).map((part) => {
			if (/^ +$/.test(part)) return document.createTextNode(part);
			const token = node("span", part);
			token.className = "create-command-token";
			return token;
		}));
		return block;
	};
	const actions = node("div");
	actions.className = `${browserUiClasses.actions} create-actions`;
	const action = (label, run, primary = false) => {
		const button = node("button");
		button.type = "button";
		button.className = browserUiClasses.action;
		if (primary) button.dataset.variant = "primary";
		const text = node("span", label);
		text.className = browserUiClasses.actionLabel;
		button.append(text);
		button.addEventListener("click", run, { signal });
		return button;
	};
	const field = (label, hint) => {
		const wrap = node("label");
		wrap.className = browserUiClasses.field;
		const name = node("span", label);
		name.className = browserUiClasses.fieldLabel;
		const input = node("input");
		input.className = browserUiClasses.fieldControl;
		input.autocomplete = "off";
		input.spellcheck = false;
		const description = node("span", hint);
		description.className = browserUiClasses.fieldDescription;
		wrap.append(name, input, description);
		return {
			wrap,
			input
		};
	};
	for (const [index, title] of titles.entries()) {
		const item = node("li");
		const button = action(`${index + 1} ${title}`, () => show(index + 1));
		button.className = `${browserUiClasses.action} ${browserUiClasses.chip} create-step`;
		item.append(button);
		steps.append(item);
		buttons.push(button);
		const panel = node("section");
		panel.className = "create-step-panel";
		panel.dataset.createStep = String(index + 1);
		panel.hidden = true;
		const heading = node("h2", title);
		heading.className = "create-step-heading";
		panel.append(heading);
		panels.push(panel);
		content.append(panel);
	}
	const choose = node("fieldset");
	choose.className = browserUiClasses.choices;
	const legend = node("legend", "Choose a connection type");
	legend.className = browserUiClasses.choicesLegend;
	choose.append(legend);
	for (const [value, label, detail] of [
		[
			"local",
			"This machine",
			"Use the door on this machine. No cloud account is needed."
		],
		[
			"firebase",
			"Firebase · Free Spark plan",
			"Follow the terminal and console steps for an existing Firebase project."
		],
		[
			"cloudflare",
			"Cloudflare · Coming soon",
			"Not available in this release."
		]
	]) {
		const row = node("label");
		row.className = browserUiClasses.choice;
		const radio = node("input");
		radio.type = "radio";
		radio.name = "create-type";
		radio.value = value;
		radio.className = browserUiClasses.radio;
		radio.checked = value === type;
		radio.disabled = value === "cloudflare";
		radio.addEventListener("change", () => {
			if (radio.checked && (value === "local" || value === "firebase")) {
				type = value;
				visited.clear();
				show(1, false);
			}
		}, { signal });
		const copy = node("span");
		const name = node("span", label);
		name.className = browserUiClasses.choiceText;
		const description = node("span", detail);
		description.className = browserUiClasses.choiceDescription;
		copy.append(name, description);
		row.append(radio, copy);
		choose.append(row);
	}
	panels[0].append(choose);
	panels[1].querySelector("h2").textContent = "Prepare Firebase";
	const card = node("div");
	card.className = `${browserUiClasses.card} create-prerequisites`;
	card.append(node("p", "Not checked: the browser cannot check these. Do them in this machine’s terminal or the Firebase console."));
	const checks = node("ul");
	checks.className = "create-checks";
	for (const [label, detail, commands] of [
		[
			"Node.js",
			"Check the installed version in this machine’s terminal.",
			[serviceCommands.nodeVersion]
		],
		[
			"Firebase CLI",
			"Check or install the supported CLI in this machine’s terminal.",
			[serviceCommands.firebaseVersion, serviceCommands.firebaseInstall]
		],
		[
			"Your Firebase account",
			"Sign in from this machine’s terminal. Firebase opens its own sign-in page.",
			[serviceCommands.firebaseLogin]
		],
		[
			"Project and Authentication",
			"In the Firebase console, choose an existing Spark project and select Authentication → Get started.",
			[]
		]
	]) {
		const row = node("li");
		const mark = node("span", "◌");
		mark.className = "create-check-mark";
		mark.setAttribute("role", "img");
		mark.setAttribute("aria-label", "Not checked");
		const copy = node("div");
		copy.append(node("strong", label), node("p", detail));
		for (const command of commands) copy.append(reference(command));
		if (!commands.length) copy.append(external("Open Firebase console", "https://console.firebase.google.com/"));
		row.append(mark, copy);
		checks.append(row);
	}
	card.append(checks);
	panels[1].append(card);
	const firebaseInputs = node("div");
	const project = field("Firebase project ID", "Use your existing project ID from the Firebase console. No account inventory is read here.");
	const region = field("Database region", "The region cannot change after authorization. Use the region of an existing database.");
	const invalid = node("p");
	invalid.className = browserUiClasses.fieldError;
	invalid.setAttribute("role", "alert");
	firebaseInputs.append(project.wrap, region.wrap, invalid, node("p", "Defaults use the (default) database, a new Web App and dedicated Hosting site, and anonymous sign-in. No paid features are set up."));
	const name = node("p");
	const localState = node("p");
	const rename = action("Name in this browser", () => options.registry.renameCurrent(rename));
	const unsubscribe = options.registry.watch((view) => {
		name.textContent = `Name in this browser: ${view.current.name}. This names the current known Remote only.`;
		localState.textContent = `This door’s observed connection: ${view.current.connection.text}.`;
	});
	signal.addEventListener("abort", unsubscribe, { once: true });
	panels[2].append(firebaseInputs, name, rename, node("p", "Names stay in this browser. A browser name is never a Firebase deployment input."));
	const review = node("div");
	panels[3].append(review);
	const result = panels[4];
	const resultCard = node("div");
	resultCard.className = `${browserUiClasses.card} create-result-card`;
	const completion = node("p");
	const saved = node("div");
	saved.className = "create-saved-facts";
	const announcement = node("p");
	announcement.className = browserUiClasses.visuallyHidden;
	announcement.setAttribute("role", "status");
	const refreshReason = node("p");
	refreshReason.id = "create-recheck-reason";
	const refreshFeedback = node("p");
	refreshFeedback.setAttribute("role", "status");
	const refresh = action("Recheck saved facts", () => {
		if (signal.aborted || rechecking || !options.canRefresh()) return;
		rechecking = true;
		updateRefresh();
		refreshFeedback.textContent = "";
		options.refresh(refresh).catch(() => {
			if (!signal.aborted) refreshFeedback.textContent = "Saved facts could not be checked. Keep the original terminal result.";
		}).finally(() => {
			if (signal.aborted) return;
			rechecking = false;
			updateRefresh();
		});
	});
	refresh.dataset.variant = "secondary";
	refresh.setAttribute("aria-describedby", refreshReason.id);
	refresh.disabled = true;
	const overview = action("Open this Remote", () => options.navigate("/remote/overview"));
	const invites = action("Invite someone", () => options.navigate("/remote/members"), true);
	resultCard.append(completion, localState, announcement, saved, refresh, refreshReason, refreshFeedback);
	result.append(resultCard);
	const back = action("Back", () => show(step === 3 && type === "local" ? 1 : step - 1));
	const next = action("Continue", () => show(step === 1 && type === "local" ? 3 : step + 1), true);
	const cancel = action("Cancel", () => options.navigate("/remote/"));
	actions.append(cancel, back, next, overview, invites);
	target.append(steps, content, actions);
	function show(selected, focus = true) {
		if (signal.aborted || selected < 1 || selected > 5 || selected === 2 && type === "local") return;
		let commands;
		if (type === "firebase" && selected >= 4) try {
			commands = deploymentCommands(project.input.value, region.input.value);
		} catch (error) {
			show(3, false);
			invalid.textContent = error instanceof Error ? error.message : "Check the project and region.";
			project.input.focus();
			return;
		}
		invalid.textContent = "";
		if (selected !== step) visited.add(step);
		step = selected;
		panels.forEach((panel, index) => {
			panel.hidden = index + 1 !== step;
		});
		buttons.forEach((button, index) => {
			const number = index + 1;
			const state = number === step ? "current" : visited.has(number) ? "done" : "upcoming";
			button.querySelector("span").textContent = `${state === "done" ? "✓ " : ""}${number} ${titles[index]}`;
			button.dataset.stepState = state;
			button.setAttribute("aria-label", `${number} ${titles[index]} · ${state}`);
			if (number === step) button.setAttribute("aria-current", "step");
			else button.removeAttribute("aria-current");
			button.disabled = type === "local" && number === 2;
		});
		firebaseInputs.hidden = type !== "firebase";
		review.replaceChildren();
		if (commands) {
			review.append(node("h3", "Preview changes and permissions · Not checked"), node("p", "Copy this preview into this machine’s terminal with the Firebase account that manages the project. It does not authorize deployment."), reference(commands.preview));
			const terminal = node("section");
			terminal.append(node("h3", "Your terminal step"), node("p", "In your own terminal, review the captured changes, including replacement of Security rules or Hosting files, and answer y/N there. The browser does not approve or execute this step."), reference(commands.deploy));
			review.append(terminal);
		} else review.append(node("p", "This machine uses its current door. No cloud account or deployment is needed. Open Overview to check the connection."));
		completion.textContent = type === "firebase" ? "Deployment completion · Not checked. A copied command or enabled layer does not verify a completed deployment. For a partial or unknown result, keep the printed record and follow the original run’s terminal guidance; do not reset it or repeat unknown work." : "No cloud deployment is involved. The connection below is the existing observation, not a new setup check.";
		saved.hidden = type !== "firebase";
		refresh.hidden = type !== "firebase";
		overview.hidden = step !== 5;
		invites.hidden = step !== 5 || type !== "firebase";
		actions.dataset.step = String(step);
		updateRefresh();
		back.hidden = step === 1;
		cancel.hidden = step !== 1;
		next.hidden = step === 5;
		if (focus) {
			const heading = panels[step - 1].querySelector("h2");
			heading.tabIndex = -1;
			heading.focus();
		}
	}
	function updateRefresh(access) {
		refresh.disabled = rechecking || access === "checking" || !options.canRefresh();
		refreshReason.textContent = !refresh.disabled ? "" : rechecking || access === "checking" ? "Checking the existing saved facts." : "Pair this browser and confirm readable management access to recheck saved facts.";
		refreshReason.hidden = type !== "firebase" || !refresh.disabled;
	}
	updateRefresh();
	show(1, false);
	return { observe(access, view) {
		if (signal.aborted) return;
		updateRefresh(access);
		renderFirestore(saved, access, view, {
			commands: false,
			announcement,
			prefix: "create"
		});
	} };
}
//#endregion
//#region src/management-page.ts
var ManagementPage = class {
	client;
	reopen;
	settings;
	devices;
	firestore;
	firestoreAccess = "checking";
	intent;
	outcome;
	access = "checking";
	accessRefusal;
	busy = false;
	removalPreview;
	get clientId() {
		return this.client.clientId;
	}
	notice = "";
	freshAttempted = false;
	cursor = null;
	firestoreRead;
	get onFirstPage() {
		return this.cursor === null;
	}
	constructor(client, reopen) {
		this.client = client;
		this.reopen = reopen;
	}
	get writable() {
		return this.writableWhenIdle && !this.busy;
	}
	/** Whether controls are available at all; a busy window only fences them, it never mutes them. */
	get writableWhenIdle() {
		return this.access === "live" && this.settings?.capabilities.settingsWrite === true && this.outcome?.state !== "unknown" && !(this.outcome?.state === "refused" && this.outcome.reason === "REMOTE_MANAGEMENT_CAPACITY");
	}
	async refresh(cursor = this.cursor) {
		if (this.busy || this.access === "lost") return;
		this.busy = true;
		this.removalPreview = void 0;
		this.accessRefusal = void 0;
		this.firestoreRead = void 0;
		this.firestore = void 0;
		this.firestoreAccess = "checking";
		const read = async () => {
			const settings = await this.client.settings();
			const devices = await this.client.devices({
				cursor,
				limit: 25
			});
			this.settings = settings;
			this.devices = devices;
			this.cursor = cursor;
			this.access = "live";
			if (this.outcome) this.describeOutcome();
			else this.notice = settings.settings.warning ?? "";
		};
		try {
			try {
				await read();
			} catch (error) {
				if (!(error instanceof RefusalError) || ![
					"REMOTE_GRANT_CHANGED",
					"REMOTE_SESSION_ENDED",
					"REMOTE_SESSION_EVICTED"
				].includes(error.code)) throw error;
				this.client = await this.reopen();
				await read();
			}
		} catch (error) {
			this.firestoreAccess = "unconfirmed";
			this.failedAccess(error);
			if (this.outcome) this.describeOutcome();
			else this.notice = "Current access could not be confirmed. Use the local CLI.";
		} finally {
			this.busy = false;
		}
	}
	/** Optional serialized observation owns no management busy window. */
	async observeFirestore() {
		const read = this.client.settings({ firestore: true });
		this.firestoreRead = read;
		try {
			const view = await read;
			if (this.firestoreRead !== read) return;
			this.firestore = view;
			this.firestoreAccess = "confirmed";
		} catch {
			if (this.firestoreRead !== read) return;
			this.firestoreAccess = "unconfirmed";
		}
	}
	async previewRemoval() {
		if (!this.writable) return;
		this.busy = true;
		this.removalPreview = void 0;
		try {
			this.removalPreview = Object.freeze(await this.client.previewRemoveOthers());
		} catch {
			this.notice = "Change refused. Check the current values or use the local CLI.";
		} finally {
			this.busy = false;
		}
	}
	async submit(intent) {
		if (!this.writable) throw new Error("Management controls are read-only.");
		this.removalPreview = void 0;
		this.intent = Object.freeze({
			...intent,
			input: Object.freeze({ ...intent.input })
		});
		this.outcome = void 0;
		this.freshAttempted = false;
		this.busy = true;
		this.notice = "";
		try {
			const frozen = this.intent;
			this.outcome = frozen.kind === "setting" ? await this.client.set(frozen.input) : frozen.kind === "rename" ? await this.client.rename(frozen.input) : frozen.kind === "talk" ? await this.client.talk(frozen.input) : frozen.kind === "remove-others" ? await this.client.removeOthers(frozen.input) : await this.client.revoke(frozen.input);
			if (this.outcome.state === "committed" && this.outcome.sessionEnded) this.access = "unconfirmed";
			if (this.outcome.state === "refused" && this.outcome.reason === "REMOTE_MANAGEMENT_READ_ONLY") this.access = "unconfirmed";
			this.describeOutcome();
		} catch (error) {
			if (!(error instanceof ClientError)) throw error;
			this.outcome = {
				operationId: this.intent.input.operationId,
				state: "unknown",
				reason: "effect_outcome_unconfirmed"
			};
			this.describeOutcome();
		} finally {
			this.busy = false;
		}
	}
	/** One explicit bounded live-grant re-admission series, then only the original read.
	* Access refusal never establishes the prior mutation's outcome.
	*/
	async recover() {
		if (!this.intent || this.busy || this.freshAttempted) return;
		this.freshAttempted = true;
		this.accessRefusal = void 0;
		this.busy = true;
		try {
			this.client = await this.reopen();
			this.access = "live";
			try {
				this.outcome = await this.client.operation(this.intent.input.operationId);
				this.describeOutcome();
			} catch {
				this.describeOutcome();
			}
		} catch (error) {
			this.failedAccess(error);
			this.describeOutcome();
		} finally {
			this.busy = false;
		}
	}
	failedAccess(error) {
		this.accessRefusal = error instanceof ReopenSessionError && error.cause instanceof RefusalError ? error.cause : error instanceof RefusalError ? error : void 0;
		this.access = accessRefused(error) ? "lost" : "unconfirmed";
	}
	get canRecover() {
		return !!this.intent && !this.busy && !this.freshAttempted && (this.outcome?.state === "unknown" || this.access !== "live");
	}
	describeOutcome() {
		if (this.outcome?.state === "committed") {
			const intent = this.intent;
			this.notice = intent?.kind === "setting" ? intent.input.setting === "open" ? "Browser opening saved." : "Session limit saved. Applies to new sessions." : intent?.kind === "rename" ? "Device renamed." : intent?.kind === "talk" ? intent.input.enabled ? "Sending enabled for this device." : "Sending disabled for this device." : intent?.kind === "remove-others" ? "The other devices were removed. This browser stays connected." : "Device removed.";
			if (this.outcome.sessionEnded) this.notice += " This session ended. Use the local CLI to continue.";
		} else if (this.outcome?.state === "unknown") this.notice = this.intent?.kind === "revoke" || this.intent?.kind === "remove-others" ? "Could not confirm the removal. Check its result before trying again." : this.freshAttempted ? "The saved value is still unconfirmed. Check with tmt remote settings or tmt remote members." : "This change could not be confirmed. Check the saved value before making another change.";
		else if (this.outcome?.state === "refused") this.notice = this.outcome.reason === "REMOTE_DEVICE_LIST_CHANGED" ? "The device list changed. Review it, then confirm again." : this.outcome.reason === "REMOTE_MANAGEMENT_CAPACITY" ? "Browser change limit reached. Use the local CLI; do not retry or reset storage." : this.outcome.reason === "REMOTE_MANAGEMENT_READ_ONLY" ? "This browser is read-only. Use the local CLI to make changes." : "Change refused. Check the current values or use the local CLI.";
	}
};
function accessRefused(error) {
	if (error instanceof ReopenSessionError) return error.reason === "revoked" || error.reason === "expired";
	return error instanceof RefusalError && ["REMOTE_DEVICE_REVOKED", "REMOTE_GRANT_EXPIRED"].includes(error.code);
}
//#endregion
//#region src/management-codec.ts
/** The signed member service accepts nonblank names of at most 64 UTF-8 bytes. */
function memberName(value) {
	return typeof value === "string" && value.trim().length > 0 && new TextEncoder().encode(value).length <= 64 && ![...value].some((character) => character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127);
}
//#endregion
//#region src/person-drafts.ts
var key = (remote) => `tmt-remote-person-drafts-v1:${remote}`;
function readPersonDrafts(storage, remote) {
	const raw = storage.getItem(key(remote));
	if (!raw) return /* @__PURE__ */ new Map();
	try {
		const data = JSON.parse(raw);
		if (!Array.isArray(data)) return /* @__PURE__ */ new Map();
		return new Map(data.filter((item) => Array.isArray(item) && item.length === 2 && typeof item[0] === "string" && item[1] !== null && typeof item[1] === "object" && typeof item[1].baseline === "string" && typeof item[1].value === "string"));
	} catch {
		return /* @__PURE__ */ new Map();
	}
}
function writePersonDraft(storage, remote, member, draft) {
	const drafts = readPersonDrafts(storage, remote);
	if (draft) drafts.set(member, draft);
	else drafts.delete(member);
	if (drafts.size) storage.setItem(key(remote), JSON.stringify([...drafts]));
	else storage.removeItem(key(remote));
}
//#endregion
//#region src/members-page.ts
/** Keeps one roster page and one explicitly opened machine page, never a second authority. */
var MembersPage = class {
	client;
	reopen;
	shouldReopen;
	endedAccess;
	roster;
	machines;
	intent;
	outcome;
	access = "checking";
	machineAccess = "idle";
	busy = false;
	cursor = null;
	generation = 0;
	disposed = false;
	constructor(client, reopen, shouldReopen = () => false, endedAccess = () => false) {
		this.client = client;
		this.reopen = reopen;
		this.shouldReopen = shouldReopen;
		this.endedAccess = endedAccess;
	}
	get onFirstPage() {
		return this.cursor === null;
	}
	get canRecover() {
		return !!this.intent && this.outcome?.state === "unknown" && !this.busy;
	}
	/** Refresh the one visible roster; an access failure must not leave old facts on screen. */
	async refresh(cursor = this.cursor) {
		if (this.busy || this.disposed) return;
		this.busy = true;
		const turn = ++this.generation;
		try {
			let roster;
			try {
				roster = await this.client.list({
					cursor,
					limit: 25
				});
			} catch (error) {
				if (!this.shouldReopen(error)) throw error;
				const client = await this.reopen();
				if (!this.current(turn)) return;
				this.client = client;
				roster = await client.list({
					cursor,
					limit: 25
				});
			}
			if (!this.current(turn)) return;
			this.roster = roster;
			this.cursor = cursor;
			this.machines = void 0;
			this.machineAccess = "idle";
			this.access = "live";
		} catch (error) {
			if (!this.current(turn)) return;
			this.clearAccess(this.endedAccess(error) ? "lost" : "unconfirmed");
		} finally {
			if (this.current(turn)) this.busy = false;
		}
	}
	/** Machine reads happen only on explicit expansion or paging for an admitted member. */
	async openMachines(memberId, cursor = null) {
		if (this.busy || this.disposed || this.access !== "live" || !this.roster?.members.some((member) => member.memberId === memberId)) return;
		this.busy = true;
		this.machines = void 0;
		this.machineAccess = "checking";
		const turn = ++this.generation;
		try {
			const machines = await this.client.machines({
				memberId,
				cursor,
				limit: 25
			});
			if (this.current(turn)) {
				this.machines = machines;
				this.machineAccess = "confirmed";
			}
		} catch (error) {
			if (this.current(turn)) {
				this.machines = void 0;
				this.machineAccess = "unconfirmed";
				if (this.endedAccess(error)) this.clearAccess("lost");
			}
		} finally {
			if (this.current(turn)) this.busy = false;
		}
	}
	/** Freeze the original before one effect; uncertain effects are never resubmitted. */
	async submit(intent) {
		if (this.busy || this.disposed || this.access !== "live" || this.outcome?.state === "unknown") throw new Error("People controls are unavailable.");
		const target = this.roster?.members.find((member) => member.memberId === intent.input.memberId);
		if (!target || target.remoteId !== intent.input.remoteId || target.revision !== intent.input.memberRevision || target.revokedAtMs !== null || intent.kind !== "rename" && !this.roster?.administrator) throw new Error("Review the current member before making this change.");
		this.intent = Object.freeze({
			...intent,
			input: Object.freeze({ ...intent.input })
		});
		this.outcome = void 0;
		this.busy = true;
		const turn = ++this.generation;
		try {
			const original = this.intent;
			const outcome = original.kind === "rename" ? await this.client.rename(original.input) : original.kind === "sending" ? await this.client.sending(original.input) : await this.client.revoke(original.input);
			if (!this.current(turn)) return;
			this.outcome = outcome;
			if (outcome.state === "committed" && outcome.sessionEnded) this.clearAccess("lost");
			if (outcome.state === "refused" && outcome.reason === "REMOTE_MANAGEMENT_READ_ONLY") this.clearAccess();
		} catch {
			if (this.current(turn)) this.outcome = this.unknown();
		} finally {
			if (this.current(turn)) this.busy = false;
		}
	}
	/** Each explicit check reopens and reads only the frozen original; never repeat its effect. */
	async recover() {
		if (!this.canRecover || this.disposed) return;
		this.busy = true;
		const turn = ++this.generation;
		try {
			const client = await this.reopen();
			if (!this.current(turn)) return;
			this.client = client;
			const outcome = await client.operation(this.intent.input.operationId);
			if (!this.current(turn)) return;
			this.outcome = this.matches(outcome) ? outcome : this.unknown();
			if (this.outcome.state === "committed" && this.outcome.sessionEnded) this.clearAccess("lost");
		} catch {
			if (this.current(turn)) this.clearAccess();
		} finally {
			if (this.current(turn)) this.busy = false;
		}
	}
	/** A lost shared Session invalidates facts without discarding an original effect. */
	invalidateAccess(ended = false) {
		if (this.disposed) return;
		if (this.busy && this.intent && !this.outcome) this.outcome = this.unknown();
		this.generation++;
		this.busy = false;
		this.clearAccess(ended ? "lost" : "unconfirmed");
	}
	dispose() {
		this.disposed = true;
		this.generation++;
		this.roster = void 0;
		this.machines = void 0;
	}
	clearAccess(access = "unconfirmed") {
		this.roster = void 0;
		this.machines = void 0;
		this.machineAccess = "idle";
		this.access = access;
	}
	current(turn) {
		return !this.disposed && this.generation === turn;
	}
	unknown() {
		return {
			operationId: this.intent.input.operationId,
			state: "unknown",
			reason: "effect_outcome_unconfirmed"
		};
	}
	matches(outcome) {
		const intent = this.intent;
		if (outcome.operationId !== intent.input.operationId) return false;
		if (outcome.state !== "committed") return true;
		if (!("member" in outcome.result)) return false;
		const member = outcome.result.member;
		if (member.remoteId !== intent.input.remoteId || member.memberId !== intent.input.memberId || member.revision < intent.input.memberRevision) return false;
		return intent.kind === "rename" ? member.displayName === intent.input.name : intent.kind === "sending" ? member.sendingEnabled === intent.input.enabled : member.revokedAtMs !== null && !member.sendingEnabled;
	}
};
/** Optional pending/unknown observations; no recorded work is not a cleanup success. */
function pendingMemberObservations(view) {
	if (!view) return "";
	const notices = [];
	if (view.invitation?.state === "current" && view.invitation.cleanup === "unconfirmed") notices.push("Invitation cleanup is unconfirmed.");
	if (view.routes.state === "unavailable") notices.push("Access update status is unavailable.");
	if (view.routes.state === "recorded") {
		if (view.routes.publicationPending) notices.push("An access update is awaiting delivery.");
		if (view.routes.removalPending) notices.push("An access removal is awaiting cleanup.");
	}
	return notices.join(" ");
}
/** Mount the People presentation around the retained member view and its original-outcome owner. */
function mountMembersPage(root, page, signal, otherBusy, changed = () => {}, observations) {
	const node = (tag, text = "") => {
		const value = document.createElement(tag);
		value.textContent = text;
		return value;
	};
	const button = (text) => {
		const value = node("button", text);
		value.type = "button";
		value.className = "tmt-ui-action";
		return value;
	};
	const compact = window.matchMedia("(max-width: 760px)");
	compact.addEventListener("change", () => render(), { signal });
	const status = node("p", "Checking people…");
	status.setAttribute("role", "status");
	const facts = node("p");
	const table = createListTable([
		{
			id: "icon",
			label: "",
			hideBelow: "phone"
		},
		{
			id: "person",
			label: "Person"
		},
		{
			id: "role",
			label: "Role"
		},
		{
			id: "sending",
			label: "Sending"
		},
		{
			id: "machines",
			label: "Machines"
		},
		{
			id: "actions",
			label: ""
		}
	], "People", "remote-people-table remote-phone-rows");
	const list = table.body;
	const footer = node("p");
	footer.className = "people-list-footer";
	const refreshButton = button("Refresh people");
	const first = button("First people");
	const more = button("Next people");
	const original = node("p");
	const recover = button("Check original outcome");
	const rows = /* @__PURE__ */ new Map();
	let localNotice = "";
	let draftsLoaded = false;
	function persistDraft(memberId, baseline, value) {
		const remoteId = page.roster?.remoteId;
		if (!remoteId) return;
		try {
			writePersonDraft(sessionStorage, remoteId, memberId, value === baseline ? void 0 : {
				baseline,
				value
			});
		} catch {
			localNotice = "This browser could not retain the unsent name after reload.";
		}
	}
	let selected;
	let machineCursor = null;
	let confirmation;
	root.replaceChildren(status, facts, refreshButton, table.root, footer, first, more, original, recover);
	function draftReason() {
		for (const row of rows.values()) if (row.input.value !== row.baseline) return "Save or restore the unsent person name first.";
		return "";
	}
	function heldBy() {
		if (page.busy) return "A request is in progress. Wait for its outcome.";
		if (page.outcome?.state === "unknown") return "A change is unconfirmed. Check its original outcome first.";
		return draftReason();
	}
	function available() {
		return page.access === "live" && !page.busy && !otherBusy() && page.outcome?.state !== "unknown";
	}
	async function run(action) {
		localNotice = "";
		let started = true;
		const pending = action();
		render();
		changed();
		try {
			await pending;
		} catch {
			started = false;
			localNotice = "This action could not start. Review current access and try again.";
		}
		if (!signal.aborted) {
			render();
			changed();
		}
		return started;
	}
	async function refresh() {
		if (page.busy || otherBusy()) return;
		if (page.roster && [...rows].some(([id, row]) => !page.roster.members.some((member) => member.memberId === id) && row.input.value !== row.baseline)) {
			localNotice = "Restore the name for the person no longer shown before refreshing.";
			render();
			return;
		}
		confirmation = void 0;
		selected = void 0;
		machineCursor = null;
		await run(() => page.refresh());
	}
	async function change(intent) {
		confirmation = void 0;
		if (await run(() => page.submit(intent)) && page.outcome?.state === "committed" && !page.outcome.sessionEnded) await run(() => page.refresh());
	}
	function confirm(memberId, kind) {
		const member = page.roster?.members.find((item) => item.memberId === memberId);
		if (!member || member.role === "owner" || !page.roster?.administrator || !available() || member.revokedAtMs !== null) return;
		const enabled = kind === "sending" ? !member.sendingEnabled : void 0;
		if (confirmation?.memberId === memberId && confirmation.kind === kind && confirmation.revision === member.revision && confirmation.enabled === enabled) {
			const input = {
				operationId: crypto.randomUUID(),
				remoteId: member.remoteId,
				memberId,
				memberRevision: member.revision
			};
			change(kind === "sending" ? {
				kind,
				input: {
					...input,
					enabled
				}
			} : {
				kind,
				input
			});
			return;
		}
		confirmation = {
			memberId,
			kind,
			revision: member.revision,
			enabled
		};
		render();
	}
	function rowFor(memberId, name) {
		const item = node("article");
		item.className = "people-person";
		const summary = node("p");
		summary.className = "tmt-ui-visually-hidden";
		const nameText = node("span", name);
		nameText.dataset.phone = "name";
		const person = node("span");
		person.className = "tmt-ui-list-leading";
		person.append(createIcon("user"));
		const roleText = node("span");
		roleText.className = "tmt-ui-chip-label";
		const roleChip = node("span");
		roleChip.className = "tmt-ui-chip remote-role";
		roleChip.append(roleText);
		const sendingText = node("span");
		sendingText.className = "tmt-ui-chip-label";
		const sendingChip = node("span");
		sendingChip.className = "tmt-ui-chip remote-stage";
		const sendingMark = node("span");
		sendingMark.className = "tmt-ui-state-mark";
		sendingMark.dataset.shape = "dot";
		sendingMark.setAttribute("aria-hidden", "true");
		sendingChip.append(sendingMark, sendingText);
		const field = node("div");
		field.className = "tmt-ui-field";
		const label = node("label", "Person name");
		label.className = "tmt-ui-field-label";
		const input = node("input");
		input.id = `people-name-${memberId}`;
		label.htmlFor = input.id;
		input.className = "tmt-ui-field-control";
		input.value = name;
		input.required = true;
		input.maxLength = 64;
		field.append(label, input);
		const nameActions = node("div");
		nameActions.className = "people-name-actions";
		const memberActions = node("div");
		memberActions.className = "people-member-actions";
		nameActions.hidden = true;
		memberActions.hidden = true;
		const save = button("Save name");
		const restore = button("Restore name");
		const send = button("Enable sending");
		const revoke = button("Remove person");
		revoke.dataset.variant = "destructive";
		const expand = button("Show machines");
		expand.dataset.variant = "ghost";
		expand.dataset.phone = "hidden";
		const machines = node("div");
		const confirmText = node("p");
		confirmText.setAttribute("role", "status");
		const cancel = button("Cancel");
		const menuHost = node("div");
		menuHost.dataset.phone = "menu";
		const menuOptions = () => ({
			label: `Actions for ${nameText.textContent}`,
			items: [
				...compact.matches ? [{
					key: "machines",
					label: "Show machines"
				}] : [],
				{
					key: "rename",
					label: "Rename"
				},
				{
					key: "sending",
					label: send.textContent ?? "Enable sending",
					disabled: send.disabled
				},
				{
					key: "revoke",
					label: "Remove person",
					disabled: revoke.disabled,
					separated: true
				}
			],
			onSelect(key) {
				if (key === "machines") expand.click();
				else if (key === "rename") {
					nameActions.hidden = false;
					input.focus();
				} else if (key === "sending" || key === "revoke") {
					memberActions.hidden = false;
					confirm(memberId, key);
				}
			}
		});
		const row = {
			root: item,
			summary,
			nameText,
			roleText,
			roleChip,
			sendingText,
			sendingChip,
			nameActions,
			memberActions,
			menu: mountBrowserMenu(menuHost, menuOptions()),
			input,
			baseline: name,
			save,
			restore,
			send,
			revoke,
			expand,
			machines,
			machineKey: "",
			confirmation: confirmText,
			cancel
		};
		nameActions.append(field, save, restore);
		memberActions.append(send, revoke);
		const tableRow = table.row([
			person,
			nameText,
			roleChip,
			sendingChip,
			expand,
			menuHost
		]);
		item.append(tableRow, summary, nameActions, memberActions, machines, confirmText, cancel);
		restore.addEventListener("click", () => {
			input.value = row.baseline;
			persistDraft(memberId, row.baseline, input.value);
			render();
		}, { signal });
		input.addEventListener("input", () => {
			persistDraft(memberId, row.baseline, input.value);
			render();
		}, { signal });
		save.addEventListener("click", () => {
			const member = page.roster?.members.find((value) => value.memberId === memberId);
			if (!member || !available() || input.value === row.baseline) return;
			if (!memberName(input.value)) {
				localNotice = "Use a nonblank name of at most 64 UTF-8 bytes without control characters.";
				render();
				return;
			}
			change({
				kind: "rename",
				input: {
					operationId: crypto.randomUUID(),
					remoteId: member.remoteId,
					memberId,
					memberRevision: member.revision,
					name: input.value
				}
			});
		}, { signal });
		send.addEventListener("click", (event) => {
			if (event.detail <= 1) confirm(memberId, "sending");
		}, { signal });
		revoke.addEventListener("click", (event) => {
			if (event.detail <= 1) confirm(memberId, "revoke");
		}, { signal });
		cancel.addEventListener("click", () => {
			confirmation = void 0;
			render();
		}, { signal });
		expand.addEventListener("click", () => {
			if (!available()) return;
			selected = memberId;
			machineCursor = null;
			run(() => page.openMachines(memberId));
		}, { signal });
		for (const action of [send, revoke]) action.addEventListener("keydown", (event) => {
			if ((event.repeat || event.isComposing) && ["Enter", " "].includes(event.key)) event.preventDefault();
			if (event.key === "Escape") {
				confirmation = void 0;
				render();
			}
		}, { signal });
		rows.set(memberId, row);
		list.append(item);
		return row;
	}
	function render() {
		if (signal.aborted) return;
		const view = page.roster;
		footer.replaceChildren(node("span", view ? `${view.members.length} ${view.members.length === 1 ? "person" : "people"} in this page` : ""), node("span", "Remove person · Enable / Disable sending in the row menu"));
		footer.hidden = !view;
		const outcome = page.outcome;
		if (localNotice) status.textContent = localNotice;
		else if (outcome?.state === "unknown") status.textContent = "This change is unconfirmed. Check its original outcome before another change.";
		else if (outcome?.state === "refused") status.textContent = "The change was refused. Review current access or use the local terminal.";
		else if (outcome?.state === "committed") status.textContent = outcome.sessionEnded ? "The change was saved and this session ended." : "The change was saved.";
		else status.textContent = page.access === "live" ? "People visible to this browser." : page.access === "checking" ? "Checking people…" : page.access === "lost" ? "This browser’s access ended. Use tmt remote members on this machine." : "People could not be confirmed. Use tmt remote members on this machine.";
		original.textContent = outcome?.state === "unknown" && page.intent ? `Original change ${page.intent.input.operationId}` : "";
		original.hidden = !original.textContent;
		recover.hidden = outcome?.state !== "unknown";
		recover.disabled = !page.canRecover || otherBusy();
		refreshButton.disabled = page.busy || otherBusy();
		first.hidden = page.onFirstPage;
		more.hidden = !view?.nextCursor;
		first.disabled = more.disabled = !available() || !!draftReason();
		facts.textContent = !view ? "" : [view.members.length ? "" : "No people in this page.", view.administrator ? "" : "Only a designated admin can change sending or remove people. Use tmt remote members on this machine."].filter(Boolean).join(" ");
		facts.hidden = !facts.textContent;
		if (observations) {
			observations.textContent = pendingMemberObservations(view);
			observations.hidden = !observations.textContent;
		}
		if (view && !draftsLoaded) {
			draftsLoaded = true;
			try {
				for (const [id, draft] of readPersonDrafts(sessionStorage, view.remoteId)) {
					const member = view.members.find((item) => item.memberId === id);
					const row = rows.get(id) ?? rowFor(id, member?.displayName ?? draft.baseline);
					row.input.value = draft.value;
					if (row.input.value !== row.baseline) row.nameActions.hidden = false;
					else persistDraft(id, row.baseline, row.input.value);
				}
			} catch {
				localNotice = "This browser could not restore an unsent name after reload.";
			}
		}
		const current = new Set(view?.members.map((member) => member.memberId) ?? []);
		for (const [id, row] of rows) if (!current.has(id)) {
			if (row.input.value === row.baseline) {
				row.menu.dispose();
				row.root.remove();
				rows.delete(id);
			} else {
				row.root.hidden = false;
				row.summary.className = "";
				row.summary.textContent = "This person is not in the current page. Your unsent name is kept; restore it to continue.";
				row.save.disabled = row.send.disabled = row.revoke.disabled = row.expand.disabled = true;
				row.restore.hidden = false;
				row.restore.disabled = page.busy || otherBusy();
			}
		}
		for (const member of view?.members ?? []) {
			const row = rows.get(member.memberId) ?? rowFor(member.memberId, member.displayName);
			row.root.hidden = false;
			row.summary.className = "tmt-ui-visually-hidden";
			if (row.input.value === row.baseline) row.input.value = member.displayName;
			row.baseline = member.displayName;
			if (row.input.value === row.baseline) persistDraft(member.memberId, row.baseline, row.input.value);
			else row.nameActions.hidden = false;
			row.summary.textContent = `${member.displayName} · ${member.role === "owner" ? "Owner" : "Member"} · Sending ${member.sendingEnabled ? "on" : "off"}${member.revokedAtMs === null ? "" : " · Revoked"}`;
			row.nameText.textContent = member.displayName;
			row.roleText.textContent = member.role === "owner" ? "Owner" : "Member";
			row.roleChip.dataset.roleTone = member.role === "owner" ? "selected" : "neutral";
			row.sendingChip.dataset.tone = member.revokedAtMs !== null ? "blocked" : member.sendingEnabled ? "working" : "neutral";
			row.sendingText.textContent = member.revokedAtMs !== null ? "Revoked" : member.sendingEnabled ? "Sending on" : "Sending off";
			const editable = available() && member.revokedAtMs === null;
			row.input.disabled = !editable;
			row.save.disabled = !editable || row.input.value === row.baseline;
			row.restore.hidden = row.input.value === row.baseline;
			row.restore.disabled = page.busy || otherBusy();
			row.input.setAttribute("aria-label", `Person name for ${member.displayName}`);
			row.save.setAttribute("aria-label", `Save name for ${member.displayName}`);
			row.restore.setAttribute("aria-label", `Restore name for ${member.displayName}`);
			row.send.setAttribute("aria-label", `${member.sendingEnabled ? "Disable" : "Enable"} sending for ${member.displayName}`);
			row.revoke.setAttribute("aria-label", `Remove ${member.displayName}`);
			row.expand.setAttribute("aria-label", `Show machines for ${member.displayName}`);
			row.send.hidden = row.revoke.hidden = member.role === "owner";
			row.send.disabled = row.revoke.disabled = member.role === "owner" || !editable || !view.administrator;
			row.send.textContent = member.sendingEnabled ? "Disable sending" : "Enable sending";
			row.expand.disabled = !available();
			row.expand.textContent = selected === member.memberId ? "Refresh machines" : "Show machines";
			const machineKey = JSON.stringify([
				selected === member.memberId,
				page.machineAccess,
				page.machines,
				machineCursor,
				available(),
				member.displayName
			]);
			if (row.machineKey !== machineKey) {
				row.machineKey = machineKey;
				row.machines.replaceChildren();
				if (selected === member.memberId && page.machineAccess === "checking") row.machines.append(node("p", "Checking machines…"));
				if (selected === member.memberId && page.machineAccess === "unconfirmed") row.machines.append(node("p", "Machine observations could not be confirmed."));
				if (selected === member.memberId && page.machines?.memberId === member.memberId) {
					if (!page.machines.machines.length) row.machines.append(node("p", "No machines in this page."));
					for (const machine of page.machines.machines) {
						const observed = machine.liveSessionCount === null ? "Activity unavailable" : machine.liveSessionCount === 0 ? "No live sessions reported" : `${machine.liveSessionCount} live ${machine.liveSessionCount === 1 ? "session" : "sessions"} reported`;
						row.machines.append(node("p", `${machine.name} · ${machine.kind}${machine.thisBrowser ? " · This browser" : ""}${machine.revoked ? " · Revoked" : machine.expiresAtMs !== null && machine.expiresAtMs <= Date.now() ? " · Expiry passed locally" : machine.grantLive ? " · Access eligible" : " · Access not active"} · ${observed} · Last used ${machine.lastUsedAtMs === null ? "not recorded" : new Date(machine.lastUsedAtMs).toLocaleString()} · Last activity ${machine.lastActivityAtMs === null ? "not observed" : new Date(machine.lastActivityAtMs).toLocaleString()}`));
					}
					if (machineCursor !== null) {
						const firstMachine = button("First machines");
						firstMachine.setAttribute("aria-label", `First machines for ${member.displayName}`);
						firstMachine.disabled = !available();
						firstMachine.addEventListener("click", () => {
							machineCursor = null;
							run(() => page.openMachines(member.memberId));
						}, { signal });
						row.machines.append(firstMachine);
					}
					if (page.machines.nextCursor) {
						const next = button("Next machines");
						next.setAttribute("aria-label", `Next machines for ${member.displayName}`);
						next.disabled = !available();
						next.addEventListener("click", () => {
							const cursor = page.machines?.nextCursor;
							if (cursor) {
								machineCursor = cursor;
								run(() => page.openMachines(member.memberId, cursor));
							}
						}, { signal });
						row.machines.append(next);
					}
				}
			}
			const pending = confirmation?.memberId === member.memberId && confirmation.revision === member.revision;
			row.confirmation.textContent = !pending ? "" : confirmation.kind === "revoke" ? `This removes ${member.displayName}'s Remote access. Press Remove person again to confirm.` : `${confirmation.enabled ? "Enable" : "Disable"} sending for ${member.displayName}. Press the same action again to confirm.`;
			row.confirmation.hidden = !pending;
			row.cancel.hidden = !pending;
			row.memberActions.hidden = !pending;
			row.menu.update({
				label: `Actions for ${member.displayName}`,
				items: [
					...compact.matches ? [{
						key: "machines",
						label: row.expand.textContent ?? "Show machines",
						disabled: row.expand.disabled
					}] : [],
					{
						key: "rename",
						label: "Rename",
						disabled: row.input.disabled
					},
					{
						key: "sending",
						label: row.send.textContent ?? "Enable sending",
						disabled: row.send.disabled
					},
					{
						key: "revoke",
						label: "Remove person",
						disabled: row.revoke.disabled,
						separated: true
					}
				],
				onSelect(key) {
					if (key === "machines") row.expand.click();
					else if (key === "rename") {
						row.nameActions.hidden = false;
						row.input.focus();
					} else if (key === "sending" || key === "revoke") {
						row.memberActions.hidden = false;
						confirm(member.memberId, key);
					}
				}
			});
		}
	}
	refreshButton.addEventListener("click", () => void refresh(), { signal });
	first.addEventListener("click", () => {
		if (!heldBy()) run(() => page.refresh(null));
	}, { signal });
	more.addEventListener("click", () => {
		const cursor = page.roster?.nextCursor;
		if (cursor && !heldBy()) run(() => page.refresh(cursor));
	}, { signal });
	recover.addEventListener("click", () => void run(() => page.recover()), { signal });
	signal.addEventListener("abort", () => {
		for (const row of rows.values()) row.menu.dispose();
		page.dispose();
	}, { once: true });
	render();
	return {
		render,
		refresh,
		heldBy
	};
}
//#endregion
//#region src/issued-invitations-page.ts
/** One view's observations and original effect. The signed service owns authority and storage. */
var IssuedInvitationsPage = class {
	client;
	reopen;
	history;
	outcome;
	secret;
	intent;
	access = "checking";
	busy = false;
	generation = 0;
	disposed = false;
	cursor = null;
	constructor(client, reopen) {
		this.client = client;
		this.reopen = reopen;
	}
	get onFirstPage() {
		return this.cursor === null;
	}
	get canRecover() {
		return !!this.intent && this.outcome?.state === "unknown" && !this.busy && !this.disposed;
	}
	async refresh(cursor = this.cursor) {
		if (this.busy || this.disposed) return;
		const turn = ++this.generation;
		this.busy = true;
		try {
			const page = await this.client.list({ cursor });
			if (!this.current(turn)) return;
			this.history = page;
			this.cursor = cursor;
			this.access = "live";
		} catch {
			if (this.current(turn)) {
				this.history = void 0;
				this.access = "unconfirmed";
			}
		} finally {
			if (this.current(turn)) this.busy = false;
		}
	}
	async create(operationId) {
		await this.submit(Object.freeze({
			kind: "create",
			operationId
		}));
	}
	async cancel(operationId, offerId) {
		await this.submit(Object.freeze({
			kind: "cancel",
			operationId,
			offerId
		}));
	}
	async submit(intent) {
		if (this.disposed || this.busy || this.access !== "live" || this.outcome?.state === "unknown") throw new Error("Invitation controls are unavailable.");
		this.secret = void 0;
		this.intent = intent;
		this.outcome = void 0;
		this.busy = true;
		const turn = ++this.generation;
		try {
			const result = intent.kind === "create" ? await this.client.create({ operationId: intent.operationId }) : await this.client.cancel({
				operationId: intent.operationId,
				offerId: intent.offerId
			});
			if (!this.current(turn)) return;
			const creation = intent.kind === "create" ? result : void 0;
			const createdSecret = creation?.state === "committed" ? creation.secret : void 0;
			if (!this.matches(result, intent)) {
				this.outcome = this.unknown(intent.operationId);
				return;
			}
			if (intent.kind === "create" && result.state === "committed") {
				this.outcome = {
					operationId: result.operationId,
					state: result.state,
					sessionEnded: result.sessionEnded,
					result: result.result
				};
				if (createdSecret) this.secret = Object.freeze({ ...createdSecret });
			} else this.outcome = result;
		} catch {
			if (this.current(turn)) this.outcome = this.unknown(intent.operationId);
		} finally {
			if (this.current(turn)) this.busy = false;
		}
	}
	/** Recovery reads the original metadata only; it never sends a new effect or recovers a secret. */
	async recover() {
		if (!this.canRecover) return;
		const intent = this.intent;
		this.secret = void 0;
		this.busy = true;
		const turn = ++this.generation;
		try {
			const client = await this.reopen();
			if (!this.current(turn)) return;
			this.client = client;
			const result = await client.operation(intent.operationId);
			if (!this.current(turn)) return;
			this.outcome = this.matches(result, intent) ? result : this.unknown(intent.operationId);
		} catch {
			if (this.current(turn)) this.access = "unconfirmed";
		} finally {
			if (this.current(turn)) this.busy = false;
		}
	}
	/** The create result is shown only until its dialog is closed. */
	clearSecret() {
		this.secret = void 0;
	}
	dispose() {
		this.disposed = true;
		this.generation++;
		this.busy = false;
		this.clearSecret();
		this.history = void 0;
	}
	current(turn) {
		return !this.disposed && this.generation === turn;
	}
	unknown(operationId) {
		return {
			operationId,
			state: "unknown",
			reason: "effect_outcome_unconfirmed"
		};
	}
	matches(result, intent) {
		if (result.operationId !== intent.operationId) return false;
		if (result.state !== "committed") return true;
		const invitation = result.result.invitation;
		return intent.kind === "create" ? invitation.state === "created" : (invitation.state === "cancelled" || invitation.state === "not-active") && invitation.offerId === intent.offerId;
	}
};
//#endregion
//#region src/invites-page.ts
var STATES = {
	open: {
		label: "Open",
		tone: "working"
	},
	"awaiting-owner": {
		label: "Awaiting you",
		tone: "waiting"
	},
	confirmed: { label: "Confirmed" },
	refused: { label: "Refused" },
	cancelled: { label: "Cancelled" },
	expired: {
		label: "Expired",
		tone: "blocked"
	},
	interrupted: {
		label: "Interrupted · needs action",
		tone: "waiting"
	},
	unconfirmed: {
		label: "Unconfirmed · needs action",
		tone: "waiting"
	}
};
var ACTIVE = ["open", "awaiting-owner"];
/** Bounded copy for the contract's create and cancel refusal reasons. */
var REFUSALS = {
	REMOTE_INVITATION_BUSY: "Another invitation request is in progress on this machine. Try again when it finishes.",
	REMOTE_INVITATION_CAPACITY: "This Remote has too many open invitations. Cancel one first.",
	REMOTE_INVITATION_STOPPED: "The invitation request stopped before it finished. Nothing was created. Try again.",
	REMOTE_INVITATION_UNAVAILABLE: "Invitations are unavailable right now. Check this Remote on this machine.",
	REMOTE_MANAGEMENT_READ_ONLY: "This browser is read-only for this Remote.",
	REMOTE_INTENT_CONFLICT: "This request conflicts with an earlier one. Refresh, then try again."
};
/** The short label names an invitation; the full offer ID stays in its title and copy action. */
function invitationLabel(offerId) {
	return `inv-${offerId.slice(0, 4)}`;
}
function ago(at, now) {
	const minutes = Math.max(0, Math.floor((now - at) / 6e4));
	if (minutes < 1) return "Just now";
	if (minutes < 60) return `${minutes} min ago`;
	if (minutes < 1440) return `${Math.floor(minutes / 60)} h ago`;
	return minutes < 2880 ? "Yesterday" : `${Math.floor(minutes / 1440)} days ago`;
}
/** Mount Invites around the retained issued-invitations model. The created code lives only in
* the open dialog and the model's memory; closing the dialog clears both. */
function mountInvitesPage(root, page, signal, options) {
	const now = options.now ?? Date.now;
	const node = (tag, text = "") => {
		const value = document.createElement(tag);
		value.textContent = text;
		return value;
	};
	const button = (text) => {
		const value = node("button", text);
		value.type = "button";
		value.className = browserUiClasses.action;
		return value;
	};
	const status = node("p", "Checking invitations…");
	status.setAttribute("role", "status");
	const facts = node("p");
	const table = createListTable([
		{
			id: "icon",
			label: "",
			hideBelow: "phone"
		},
		{
			id: "invitation",
			label: "Invitation"
		},
		{
			id: "status",
			label: "Status"
		},
		{
			id: "issued",
			label: "Issued",
			hideBelow: "phone"
		},
		{
			id: "expires",
			label: "Expires"
		},
		{
			id: "actions",
			label: ""
		}
	], "Invitations", "remote-invites-table remote-phone-rows");
	const footer = node("p");
	footer.className = "invites-list-footer";
	const first = button("First invitations");
	const more = button("Next invitations");
	const original = node("p");
	const recover = button("Check original outcome");
	root.replaceChildren(status, facts, table.root, footer, first, more, original, recover);
	let menus = [];
	let localNotice = "";
	let dialog;
	function heldBy() {
		if (page.busy) return "An invitation request is in progress. Wait for its outcome.";
		if (page.outcome?.state === "unknown") return "An invitation change is unconfirmed. Check its original outcome first.";
		return "";
	}
	function available() {
		return page.access === "live" && !page.busy && !options.otherBusy() && page.outcome?.state !== "unknown";
	}
	async function run(action) {
		const pending = action();
		render();
		options.changed();
		try {
			await pending;
		} catch {
			localNotice = "This action could not start. Review current access and try again.";
		}
		if (!signal.aborted) {
			render();
			options.changed();
		}
	}
	async function refresh(cursor) {
		if (page.busy || options.otherBusy()) return;
		localNotice = "";
		await run(() => page.refresh(cursor));
	}
	async function invite() {
		if (!available()) return;
		localNotice = "";
		await run(() => page.create(crypto.randomUUID()));
		if (page.secret) showSecret(page.secret);
		if (page.outcome?.state === "committed") await refresh(null);
	}
	async function cancel(offerId) {
		if (!available()) return;
		localNotice = "";
		await run(() => page.cancel(crypto.randomUUID(), offerId));
		if (page.outcome?.state === "committed") await refresh(null);
	}
	function copyRow(text, label) {
		const row = node("div");
		row.className = "invite-secret-row";
		const value = node("code", text);
		value.className = "invite-secret";
		value.title = text;
		const copy = button(label);
		const feedback = node("span");
		feedback.setAttribute("role", "status");
		copy.addEventListener("click", async () => {
			try {
				await navigator.clipboard.writeText(text);
				feedback.textContent = "Copied.";
			} catch {
				feedback.textContent = "Copy failed. Select the text and copy it manually.";
			}
		});
		row.append(value, copy, feedback);
		return row;
	}
	function showSecret(secret) {
		dialog?.close();
		const box = node("dialog");
		box.className = `${browserUiClasses.dialog} invite-dialog`;
		const header = node("header");
		header.className = browserUiClasses.dialogHeader;
		const heading = node("h2", "Invitation created");
		heading.id = "invite-dialog-heading";
		box.setAttribute("aria-labelledby", heading.id);
		const close = node("button");
		close.type = "button";
		close.className = "tmt-ui-icon-action";
		close.setAttribute("aria-label", "Close");
		close.append(createIcon("x"));
		header.append(heading, close);
		const body = node("div");
		body.className = browserUiClasses.dialogBody;
		const expiry = page.outcome?.state === "committed" && "expiresAtMs" in page.outcome.result.invitation ? Math.max(1, Math.round((page.outcome.result.invitation.expiresAtMs - now()) / 6e4)) : void 0;
		body.append(node("p", `Give this code or link to one person. It works once${expiry ? ` and expires in ${expiry} min` : ""}.`), copyRow(secret.code, "Copy code"), ...secret.link ? [copyRow(secret.link, "Copy link")] : [], Object.assign(node("p", "This code is shown only now. After you close this, it can't be shown again."), { className: "invite-secret-note" }));
		const actions = node("footer");
		actions.className = browserUiClasses.dialogActions;
		const done = button("Done");
		done.dataset.variant = "primary";
		actions.append(done);
		box.append(header, body, actions);
		for (const end of [close, done]) end.addEventListener("click", () => box.close());
		box.addEventListener("close", () => {
			box.remove();
			page.clearSecret();
			if (dialog === box) dialog = void 0;
			render();
		}, { once: true });
		dialog = box;
		document.body.append(box);
		box.showModal();
	}
	signal.addEventListener("abort", () => dialog?.close(), { once: true });
	recover.addEventListener("click", () => void run(() => page.recover()), { signal });
	first.addEventListener("click", () => void refresh(null), { signal });
	more.addEventListener("click", () => void refresh(page.history?.nextCursor ?? null), { signal });
	function outcomeText() {
		const outcome = page.outcome;
		if (!outcome || !page.intent) return "";
		if (outcome.state === "unknown") return page.intent.kind === "create" ? "Creating this invitation is unconfirmed. Check its original outcome before inviting again." : "Cancelling this invitation is unconfirmed. Check its original outcome first.";
		if (outcome.state === "refused") return REFUSALS[outcome.reason] ?? "The request was refused. Review current access or use tmt remote invite on this machine.";
		const invitation = outcome.result.invitation;
		if (invitation.state === "created") return page.secret ? "Invitation created." : "Invitation created. Its code is no longer shown; invite again for a new one.";
		if (invitation.state === "not-active") return "That invitation was no longer open. Nothing changed.";
		if (invitation.state === "cancelled") return "Invitation cancelled. Anyone who already joined keeps access.";
		return "";
	}
	function render() {
		if (signal.aborted) return;
		const view = page.history;
		status.textContent = localNotice || outcomeText() || (page.access === "live" ? "" : page.access === "checking" ? "Checking invitations…" : "Invitations could not be confirmed. Use tmt remote invite on this machine.");
		status.hidden = !status.textContent;
		facts.textContent = view && !view.invitations.length ? "No invitations in the last 7 days." : "";
		facts.hidden = !facts.textContent;
		table.root.hidden = !view?.invitations.length;
		original.textContent = page.outcome?.state === "unknown" && page.intent ? `Original change ${page.intent.operationId}` : "";
		original.hidden = !original.textContent;
		recover.hidden = page.outcome?.state !== "unknown";
		recover.disabled = !page.canRecover || options.otherBusy();
		first.hidden = page.onFirstPage;
		more.hidden = !view?.nextCursor;
		first.disabled = more.disabled = !available();
		footer.replaceChildren(node("span", view ? `${view.invitations.length} ${view.invitations.length === 1 ? "invitation" : "invitations"} · newest first` : ""), node("span", "Cancel an open invitation in the row menu"));
		footer.hidden = !view?.invitations.length;
		menus.forEach((menu) => menu.dispose());
		menus = [];
		table.body.replaceChildren(...(view?.invitations ?? []).map((invitation) => {
			const leading = node("span");
			leading.className = browserUiClasses.listLeading;
			leading.append(createIcon("link"));
			const title = node("div");
			title.className = browserUiClasses.listTitle;
			title.dataset.phone = "name";
			const label = node("span", invitationLabel(invitation.offerId));
			label.className = `${browserUiClasses.listLabel} invite-id`;
			label.title = invitation.offerId;
			title.append(label);
			if (invitation.state === "awaiting-owner") {
				const hint = node("span", "Someone used this code · confirm or refuse in the row menu");
				hint.className = browserUiClasses.listDescription;
				title.append(hint);
			}
			const state = STATES[invitation.state];
			const chip = node("span");
			chip.className = `${browserUiClasses.chip} remote-stage`;
			if (state.tone) chip.dataset.tone = state.tone;
			const mark = node("span");
			mark.className = browserUiClasses.stateMark;
			mark.dataset.shape = "dot";
			mark.setAttribute("aria-hidden", "true");
			const text = node("span", state.label);
			text.className = browserUiClasses.chipLabel;
			chip.append(mark, text);
			const active = ACTIVE.includes(invitation.state) && invitation.expiresAtMs > now();
			const expires = node("span", active ? `in ${Math.max(1, Math.ceil((invitation.expiresAtMs - now()) / 6e4))} min` : "—");
			const actions = node("div");
			actions.className = browserUiClasses.listActions;
			actions.dataset.phone = "menu";
			menus.push(mountBrowserMenu(actions, {
				label: `Actions for ${label.textContent}`,
				items: [
					...invitation.state === "awaiting-owner" ? [{
						key: "confirm",
						label: "Confirm or refuse…"
					}] : [],
					{
						key: "copy",
						label: "Copy invitation ID"
					},
					...ACTIVE.includes(invitation.state) ? [{
						key: "cancel",
						label: "Cancel invitation",
						disabled: !available(),
						separated: true
					}] : []
				],
				onSelect(key) {
					if (key === "confirm") options.confirm();
					else if (key === "cancel") cancel(invitation.offerId);
					else navigator.clipboard.writeText(invitation.offerId).catch(() => {});
				}
			}));
			const row = table.row([
				leading,
				title,
				chip,
				node("span", ago(invitation.issuedAtMs, now())),
				expires,
				actions
			]);
			row.dataset.offer = invitation.offerId;
			return row;
		}));
	}
	render();
	return {
		render,
		refresh: () => refresh(null),
		invite,
		heldBy
	};
}
/**
* Decides when a busy mark is visible: only after busy has lasted the delay,
* then for at least the hold, so short work never flashes a mark.
*/
function createBusyMarkGate(onChange) {
	let busy = false;
	let shown = false;
	let shownAt = 0;
	let timer;
	const clear = () => {
		if (timer !== void 0) clearTimeout(timer);
		timer = void 0;
	};
	const show = (value) => {
		shown = value;
		if (value) shownAt = Date.now();
		onChange(value);
	};
	return {
		shown: () => shown,
		set(next) {
			if (next === busy) return;
			busy = next;
			clear();
			if (busy) {
				if (!shown) timer = setTimeout(() => show(true), 400);
				return;
			}
			if (!shown) return;
			const remaining = shownAt + 400 - Date.now();
			if (remaining <= 0) show(false);
			else timer = setTimeout(() => show(false), remaining);
		},
		/** Stops pending timers; a later busy start begins a fresh delay. */
		dispose() {
			clear();
			busy = false;
		}
	};
}
//#endregion
//#region src/settings-page.ts
/** The Remote management tabs (Members, My devices, Settings and Service & upgrade): run once per mount of
* their markup. This bundle is loaded apart from the SDK, so the app hands it its part of the
* shell and the route's one remotes controller. */
async function settingsPage(shell, registry, initial) {
	function element(id) {
		const found = document.getElementById(id);
		if (!found) throw new Error(`Settings page lacks #${id}.`);
		return found;
	}
	const opening = element("opening");
	const mode = element("limit-mode");
	const custom = element("limit-custom");
	const devices = element("devices");
	const refresh = element("refresh");
	const removeOthers = element("remove-others");
	const more = element("more");
	const first = element("first");
	const rows = /* @__PURE__ */ new Map();
	let initialized = false;
	let choices;
	let confirmation;
	function targetSignature(target) {
		return JSON.stringify([
			target.name,
			target.revoked,
			target.talkEnabled,
			target.thisBrowser
		]);
	}
	function cancelConfirmation(refocus = false) {
		if (!confirmation) return;
		const { button, clientId } = confirmation;
		const panel = rows.get(clientId)?.querySelector("[data-confirmation]");
		if (panel) panel.hidden = true;
		button.removeAttribute("aria-describedby");
		button.setAttribute("aria-expanded", "false");
		confirmation = void 0;
		if (refocus && button.isConnected && !button.disabled) button.focus({ preventScroll: true });
	}
	/** A second deliberate press confirms only the same currently admitted presentation. */
	function confirmDevice(button, target, kind) {
		page.removalPreview = void 0;
		const signature = targetSignature(target);
		if (confirmation?.button === button && confirmation.signature === signature) {
			cancelConfirmation();
			return true;
		}
		cancelConfirmation();
		const panel = rows.get(target.clientId).querySelector("[data-confirmation]");
		const consequence = panel.querySelector("p");
		consequence.textContent = kind === "talk" ? `${target.name} will be able to send to its permitted agents. Press Enable sending again to confirm. You can disable sending later.` : target.thisBrowser ? `This ends Remote access for this browser (${target.name}). Press Remove again to confirm.` : `This ends Remote access for ${target.name}. Press Remove again to confirm.`;
		panel.hidden = false;
		button.setAttribute("aria-describedby", consequence.id);
		button.setAttribute("aria-expanded", "true");
		confirmation = {
			button,
			clientId: target.clientId,
			signature
		};
		return false;
	}
	let page;
	let people;
	let peopleView;
	let invitationAttached = false;
	let invites;
	let invitesView;
	let invitesUnread = false;
	const inviteButton = shell ? document.getElementById("remote-invite") : null;
	if (inviteButton && !inviteButton.querySelector(":scope > svg")) inviteButton.prepend(createIcon("link"));
	/** Only the connected owner browser reads or changes issued invitations. */
	const mayInvite = () => !!page && page.access === "live" && people?.roster?.administrator === true;
	let invitationSession;
	const registryState = registry ?? remoteRegistryPage(void 0, shell);
	const invitationRoot = shell && document.getElementById("owner-invitation");
	const ownerInvitations = invitationRoot ? invitationPage(invitationRoot, shell?.signal, void 0, () => !!page && (page.access !== "live" || people?.roster?.administrator !== true || page.busy || page.outcome?.state === "unknown" || !!people?.busy || people?.outcome?.state === "unknown"), () => {
		if (!page) return;
		render();
		if (remoteItem(location.pathname)?.id === "members" && page.access === "live" && peopleView && !people?.roster && !people?.busy && !ownerInvitations?.heldBy()) peopleView.refresh();
	}) : void 0;
	const otherHold = () => ownerInvitations?.heldBy() || invitesView?.heldBy() || (people?.busy ? "A People request is in progress." : people?.outcome?.state === "unknown" ? "A People change is unconfirmed. Check its original outcome first." : "");
	const guideTarget = shell && document.getElementById("remote-create");
	const guide = shell && guideTarget ? createPage(guideTarget, {
		signal: shell.signal,
		registry: registryState,
		navigate: (path) => shell.navigate(new URL(path, location.href).href),
		canRefresh: () => !!page && page.access === "live",
		refresh: (owner) => run(() => refreshView(), owner)
	}) : void 0;
	function renderService() {
		renderFirestore(element("firestore-content"), page.firestoreAccess, page.firestore);
		guide?.observe(page.firestoreAccess, page.firestore);
	}
	registryState.blockedBy(() => {
		if (!page) return "";
		if (page.busy) return "A request is in progress. Wait for its outcome.";
		if (page.outcome?.state === "unknown") return "Another change is still unconfirmed. Check the saved value first.";
		const peopleReason = peopleView?.heldBy() || ownerInvitations?.heldBy() || invitesView?.heldBy();
		if (peopleReason) return peopleReason;
		for (const device of page.devices?.devices ?? []) {
			const name = rows.get(device.clientId)?.querySelector("input");
			if (name && name.value !== device.name) return "Save or restore the unsent device name before changing pages.";
		}
		return "";
	});
	/** Format fixed CLI hints as literal text, never interpret notice content as HTML. */
	function commandNotice(target, text) {
		target.replaceChildren();
		for (const part of text.split(/(tmt remote (?:pair|devices|settings))/g)) if (/^tmt remote (?:pair|devices|settings)$/.test(part)) {
			const code = document.createElement("code");
			code.className = "tmt-ui-code";
			code.textContent = part;
			target.append(code);
		} else target.append(document.createTextNode(part));
	}
	function capacityCopy(refusal) {
		return refusal?.code === "REMOTE_SESSION_LIMIT" ? "This browser has reached your machine's connection limit. Close another tab, or change the limit on your machine. Your pairing is unchanged." : void 0;
	}
	function openingChanged() {
		return !!page.settings && opening.value === "on" !== page.settings.settings.open;
	}
	function limitChanged() {
		if (!page.settings || mode.value === "default") return false;
		const admitted = page.settings.settings;
		const value = mode.value === "off" ? null : custom.value;
		return admitted.sessionsPerDeviceSource !== "settings.json" || value !== admitted.sessionsPerDevice;
	}
	/** Busy is not disabled: only an unavailable control is disabled. A busy one keeps its look and
	* focus and is fenced by aria-disabled; every activation handler already refuses while busy. */
	function fence(button, unavailable) {
		button.disabled = unavailable || !!otherHold();
		if (page.busy && !unavailable) button.setAttribute("aria-disabled", "true");
		else button.removeAttribute("aria-disabled");
	}
	/** The action that carries the busy mark: the one the current write started from. */
	let markOwner;
	function markSlot(button) {
		let slot = button.querySelector(":scope > .tmt-ui-action-mark");
		if (!slot) {
			slot = createActionMark();
			button.prepend(slot);
		}
		return slot;
	}
	/** The shared delay and hold: nothing shows for the first 400 ms, then the mark stays 400 ms. */
	const busyMark = createBusyMarkGate((shown) => {
		if (markOwner) markSlot(markOwner).dataset.busy = String(shown);
	});
	function syncBusyMark() {
		for (const button of document.querySelectorAll("button.tmt-ui-action")) if (!button.closest("#remotes, #remote-head, #remote-switcher, #remote-context, #remote-local-name, #remote-locked, #remote-create, #remote-join-guide, #owner-invitation, #people")) markSlot(button);
		busyMark.set(page.busy);
	}
	function saveState(form, hint, changed, valid) {
		const button = element(form).querySelector("button[type=submit]");
		fence(button, !page.writableWhenIdle || !changed || !valid);
		element(hint).hidden = !page.writable || changed;
		const described = button.getAttribute("aria-describedby");
		if (!element(hint).hidden) button.setAttribute("aria-describedby", [described, hint].filter(Boolean).join(" "));
	}
	function recoverOriginal(event) {
		if (page.busy) return;
		run(async () => {
			await page.recover();
			if (page.access === "live") await refreshView();
		}, event.currentTarget);
	}
	/** Mount an empty live region before a device can originate a change. Never move it. */
	function deviceFeedback(clientId) {
		const feedback = document.createElement("div");
		feedback.className = "change-feedback";
		feedback.dataset.feedback = clientId;
		const status = document.createElement("p");
		status.dataset.outcomeSlot = "";
		status.setAttribute("role", "status");
		status.setAttribute("aria-live", "polite");
		status.setAttribute("aria-atomic", "true");
		const original = document.createElement("p");
		original.dataset.original = "";
		original.hidden = true;
		const recover = document.createElement("button");
		recover.type = "button";
		recover.className = "tmt-ui-action";
		recover.dataset.recover = "";
		recover.hidden = true;
		const label = document.createElement("span");
		label.className = "tmt-ui-action-label";
		label.textContent = "Check the saved value";
		recover.append(label);
		recover.addEventListener("click", recoverOriginal);
		feedback.append(status, original, recover);
		return feedback;
	}
	function renderOutcome() {
		const origin = page.intent?.kind === "setting" ? page.intent.input.setting : page.intent?.kind === "remove-others" ? void 0 : page.intent?.input.clientId;
		const active = origin && [...document.querySelectorAll("[data-feedback]")].find((slot) => slot.dataset.feedback === origin) || document.querySelector("[data-feedback=\"shared\"]");
		for (const slot of document.querySelectorAll("[data-feedback]")) if (slot !== active) {
			const status = slot.querySelector("[data-outcome-slot]");
			if (status.textContent) commandNotice(status, "");
		}
		for (const slot of document.querySelectorAll("[data-feedback]")) {
			const status = slot.querySelector("[data-outcome-slot]");
			const original = slot.querySelector("[data-original]");
			const recover = slot.querySelector("[data-recover]");
			const selected = slot === active;
			status.id = selected ? "outcome" : "";
			original.id = selected ? "original" : "";
			recover.id = selected ? "recover" : "";
			const text = selected ? page.notice : "";
			if (status.textContent !== text) commandNotice(status, text);
			status.dataset.state = selected ? page.outcome?.state ?? "pending" : "";
			slot.dataset.tone = page.busy ? "waiting" : page.outcome?.state === "committed" ? "working" : "blocked";
			original.textContent = selected && page.intent && (page.outcome?.state === "unknown" || page.outcome?.state === "refused") ? `Original change ${page.intent.input.operationId}` : "";
			original.hidden = !original.textContent;
			recover.hidden = !selected || !page.canRecover;
			fence(recover, false);
			recover.setAttribute("aria-busy", String(page.busy));
		}
	}
	function render() {
		if (people && page.access === "lost" && people.access !== "lost") people.invalidateAccess(true);
		else if (people && page.access !== "live" && ["checking", "live"].includes(people.access)) people.invalidateAccess();
		const owner = mayInvite();
		if (!owner && invitationAttached) {
			invitationAttached = false;
			ownerInvitations?.();
		} else if (owner && !invitationAttached && invitationSession) {
			invitationAttached = true;
			ownerInvitations?.(invitationSession);
		}
		if (!owner && invitationRoot && !ownerInvitations?.heldBy()) {
			const status = invitationRoot.querySelector("#invitation-status");
			if (status) status.textContent = page.access === "live" && people?.access === "live" && people.roster?.administrator === false ? "To confirm an invitation in a browser, use the owner browser set up on this machine. Use tmt remote members on this machine." : "Invitation access is unconfirmed. Use tmt remote members on this machine.";
		}
		ownerInvitations?.update();
		peopleView?.render();
		invitesView?.render();
		readInvites();
		const invitesContent = document.getElementById("invites-content");
		if (invitesContent) invitesContent.hidden = people?.roster?.administrator === false;
		if (inviteButton) {
			inviteButton.hidden = !owner;
			fence(inviteButton, !owner || !!invites?.busy || !!otherHold());
		}
		registryState(page.access === "live" ? "Connected" : page.access === "lost" ? "Access ended" : page.access === "checking" ? "Checking" : "Unconfirmed", void 0, managementRole(page.access, page.settings?.capabilities.settingsWrite) ?? null);
		if (confirmation) {
			const target = page.devices?.devices.find((device) => device.clientId === confirmation.clientId);
			if (!page.writable || !target || targetSignature(target) !== confirmation.signature) cancelConfirmation();
		}
		renderService();
		const capacity = capacityCopy(page.accessRefusal);
		const access = capacity ?? {
			checking: "Checking current access…",
			live: "Current browser access confirmed.",
			lost: "Current browser access refused.",
			unconfirmed: "Current browser access unconfirmed."
		}[page.access];
		if (element("access").textContent !== access) element("access").textContent = access;
		const announcement = capacity ?? {
			checking: "",
			live: "Browser access confirmed",
			lost: "Access ended",
			unconfirmed: "Access unconfirmed"
		}[page.access];
		if (announcement && element("access-announcement").textContent !== announcement) element("access-announcement").textContent = announcement;
		const editable = page.access === "live" && page.settings?.capabilities.settingsWrite === true;
		element("read-only").hidden = editable || page.access !== "live";
		const designation = element("designation-command");
		designation.hidden = editable || page.access !== "live";
		if (!designation.hidden) element("designation-code").textContent = `tmt remote members designate ${page.clientId}`;
		element("access-notice").dataset.tone = page.access === "live" ? editable ? "working" : "review" : page.access === "checking" ? "waiting" : "blocked";
		const reason = capacity ?? (otherHold() || (!editable ? page.access === "live" ? "" : "Current browser access could not be confirmed." : page.outcome?.state === "unknown" ? "Another change is still unconfirmed. Check the saved value first." : page.outcome?.state === "refused" && page.outcome.reason === "REMOTE_MANAGEMENT_CAPACITY" ? "Browser change limit reached. Use the local CLI; do not retry or reset storage." : ""));
		element("controls-reason").textContent = reason;
		element("controls-reason").hidden = !reason;
		for (const button of document.querySelectorAll("button")) {
			if (button.closest("#remotes, #remote-head, #remote-switcher, #remote-context, #remote-local-name, #remote-locked, #owner-invitation, #people")) continue;
			if (reason) button.setAttribute("aria-describedby", "controls-reason");
			else button.removeAttribute("aria-describedby");
			button.setAttribute("aria-busy", String(page.busy));
		}
		const openingForm = element("opening-form");
		if (editable) openingForm.removeAttribute("aria-describedby");
		else openingForm.setAttribute("aria-describedby", "read-only");
		element("limit-form").setAttribute("aria-describedby", editable ? "limit-help" : "limit-help read-only");
		if (page.settings) {
			const value = page.settings.settings;
			element("opening-value").textContent = `${value.open ? "On" : "Off"} · ${value.source}`;
			element("limit-value").textContent = `${value.sessionsPerDevice ?? "Off (unlimited)"} · ${value.sessionsPerDeviceSource}`;
			const warning = element("warning");
			warning.textContent = value.warning ?? "";
			warning.hidden = value.warning === null;
			if (!initialized) {
				opening.value = value.open ? "on" : "off";
				mode.value = value.sessionsPerDeviceSource === "default" ? "default" : value.sessionsPerDevice === null ? "off" : "custom";
				custom.value = value.sessionsPerDevice ?? "";
				initialized = true;
			}
		}
		custom.required = mode.value === "custom";
		for (const field of [
			opening,
			mode,
			custom
		]) field.disabled = !editable || field === custom && mode.value !== "custom";
		for (const button of document.querySelectorAll("form button:not(.tmt-ui-menu-trigger):not([role=\"option\"])")) fence(button, !page.writableWhenIdle);
		if (page.settings) {
			choices ??= [settingsChoice(opening, element("opening-options")), settingsChoice(mode, element("limit-mode-options"))];
			for (const update of choices) update();
		}
		saveState("opening-form", "opening-unchanged", openingChanged(), opening.checkValidity());
		saveState("limit-form", "limit-unchanged", limitChanged(), mode.value !== "custom" || custom.checkValidity());
		removeOthers.hidden = page.removalPreview?.count === 0 || page.onFirstPage && page.devices?.nextCursor === null && page.devices.devices.every((device) => device.clientId === page.clientId);
		fence(removeOthers, !page.writableWhenIdle || page.removalPreview?.count === 0);
		const preview = page.removalPreview;
		const panel = element("remove-others-confirmation");
		panel.hidden = !preview || preview.count === 0 || !page.writable;
		removeOthers.setAttribute("aria-expanded", String(!panel.hidden));
		if (!panel.hidden && preview) {
			element("remove-others-consequence").textContent = `This ends Remote access for ${preview.count} other ${preview.count === 1 ? "device" : "devices"}. This browser stays connected. Press Remove the others again to confirm.`;
			removeOthers.setAttribute("aria-describedby", "remove-others-consequence");
		}
		fence(refresh, false);
		more.hidden = !page.devices?.nextCursor;
		fence(more, false);
		first.hidden = page.onFirstPage;
		fence(first, false);
		if (page.devices) {
			const current = new Set(page.devices.devices.map((device) => device.clientId));
			for (const [id, row] of rows) if (!current.has(id)) {
				row.remove();
				rows.delete(id);
			}
			for (const device of page.devices.devices) {
				let row = rows.get(device.clientId);
				if (!row) {
					row = document.createElement("div");
					row.className = "device tmt-ui-list-row";
					const summary = document.createElement("p");
					summary.className = "device-summary";
					const form = document.createElement("form");
					const label = document.createElement("label");
					const name = document.createElement("input");
					name.id = `name-${device.clientId}`;
					name.className = "tmt-ui-field-control";
					label.id = `${name.id}-label`;
					label.className = "tmt-ui-field-label";
					name.setAttribute("aria-labelledby", label.id);
					name.value = device.name;
					name.required = true;
					name.maxLength = 64;
					label.htmlFor = name.id;
					label.textContent = "Device name";
					const save = document.createElement("button");
					save.type = "submit";
					save.className = "tmt-ui-action";
					const saveLabel = document.createElement("span");
					saveLabel.className = "tmt-ui-action-label";
					saveLabel.textContent = "Rename";
					save.append(saveLabel);
					const revoke = document.createElement("button");
					revoke.type = "button";
					revoke.dataset.action = "revoke";
					revoke.className = "tmt-ui-action";
					revoke.dataset.variant = "destructive";
					const revokeLabel = document.createElement("span");
					revokeLabel.className = "tmt-ui-action-label";
					revokeLabel.textContent = "Remove";
					revoke.append(revokeLabel);
					revoke.addEventListener("click", (event) => {
						if (event.detail > 1 || !page.writable) return;
						const target = page.devices?.devices.find((item) => item.clientId === device.clientId);
						if (!target || target.revoked) return;
						if (!confirmDevice(revoke, target, "revoke")) return;
						change({
							kind: "revoke",
							input: {
								operationId: crypto.randomUUID(),
								clientId: device.clientId
							}
						}, revoke);
					});
					form.addEventListener("submit", (event) => {
						event.preventDefault();
						if (!page.writable) return;
						change({
							kind: "rename",
							input: {
								operationId: crypto.randomUUID(),
								clientId: device.clientId,
								name: name.value
							}
						}, save);
					});
					const field = document.createElement("div");
					field.className = "tmt-ui-field";
					field.append(label, name);
					const talk = document.createElement("button");
					talk.type = "button";
					talk.className = "tmt-ui-action";
					talk.dataset.action = "talk";
					const talkLabel = document.createElement("span");
					talkLabel.className = "tmt-ui-action-label";
					talk.append(talkLabel);
					talk.addEventListener("click", (event) => {
						if (event.detail > 1 || !page.writable) return;
						const target = page.devices?.devices.find((item) => item.clientId === device.clientId);
						if (!target || target.revoked) return;
						if (!target.talkEnabled && !confirmDevice(talk, target, "talk")) return;
						change({
							kind: "talk",
							input: {
								operationId: crypto.randomUUID(),
								clientId: target.clientId,
								enabled: !target.talkEnabled
							}
						}, talk);
					});
					form.append(field, save, talk, revoke);
					const panel = document.createElement("div");
					panel.className = browserUiClasses.notice;
					panel.dataset.confirmation = "";
					panel.id = `confirm-${device.clientId}`;
					panel.hidden = true;
					const consequence = document.createElement("p");
					consequence.id = `${panel.id}-consequence`;
					consequence.className = browserUiClasses.noticeBody;
					consequence.setAttribute("role", "status");
					const cancel = document.createElement("button");
					cancel.type = "button";
					cancel.className = "tmt-ui-action";
					const cancelLabel = document.createElement("span");
					cancelLabel.className = "tmt-ui-action-label";
					cancelLabel.textContent = "Cancel";
					cancel.append(cancelLabel);
					cancel.addEventListener("click", () => cancelConfirmation(true));
					const actions = document.createElement("div");
					actions.className = browserUiClasses.noticeActions;
					actions.append(cancel);
					panel.append(consequence, actions);
					for (const button of [talk, revoke]) {
						button.setAttribute("aria-controls", panel.id);
						button.setAttribute("aria-expanded", "false");
						button.addEventListener("keydown", (event) => {
							if ((event.repeat || event.isComposing) && ["Enter", " "].includes(event.key)) event.preventDefault();
						});
					}
					row.addEventListener("keydown", (event) => {
						if (event.key === "Escape" && confirmation?.clientId === device.clientId) {
							event.preventDefault();
							cancelConfirmation(true);
						}
					});
					const disabledReason = document.createElement("p");
					disabledReason.id = `device-reason-${device.clientId}`;
					disabledReason.className = "tmt-ui-field-description";
					row.append(summary, form, panel, deviceFeedback(device.clientId), disabledReason);
					rows.set(device.clientId, row);
					devices.append(row);
				}
				row.querySelector(".device-summary").textContent = `${device.name}${device.thisBrowser ? " · This device" : ""} · ${device.kind} · ${device.revoked ? "Revoked" : `Paired · Sending ${device.talkEnabled ? "on" : "off"}`} · ${device.liveSessionCount} live ${device.liveSessionCount === 1 ? "session" : "sessions"} · Last used ${device.lastUsedAtMs === null ? "Not recorded" : new Date(device.lastUsedAtMs).toLocaleString()}`;
				row.querySelector("[data-action=\"talk\"] .tmt-ui-action-label").textContent = device.talkEnabled ? "Disable sending" : "Enable sending";
				row.querySelector("form").setAttribute("aria-label", `Device ${device.name}`);
				const name = row.querySelector("input");
				name.removeAttribute("aria-labelledby");
				name.setAttribute("aria-label", `Device name for ${device.name}`);
				row.querySelector("button[type=submit]").setAttribute("aria-label", device.thisBrowser ? `Rename this device (${device.name})` : `Rename ${device.name}`);
				row.querySelector("[data-action=\"talk\"]").setAttribute("aria-label", `${device.talkEnabled ? "Disable" : "Enable"} sending for ${device.name}`);
				row.querySelector("button[data-action=\"revoke\"]").setAttribute("aria-label", `Remove ${device.name}`);
				name.disabled = !editable || device.revoked;
				if (editable) name.removeAttribute("aria-describedby");
				else name.setAttribute("aria-describedby", "read-only");
				const disabledReason = element(`device-reason-${device.clientId}`);
				disabledReason.textContent = page.outcome?.state === "unknown" && page.intent?.kind !== "setting" && page.intent?.kind !== "remove-others" && page.intent?.input.clientId === device.clientId ? "" : device.revoked ? "This device is revoked." : reason;
				disabledReason.hidden = !disabledReason.textContent;
				for (const button of row.querySelectorAll("form button")) {
					fence(button, !page.writableWhenIdle || device.revoked);
					if (disabledReason.textContent) button.setAttribute("aria-describedby", disabledReason.id);
					else if (confirmation?.button === button) button.setAttribute("aria-describedby", `confirm-${device.clientId}-consequence`);
					else button.removeAttribute("aria-describedby");
					button.setAttribute("aria-busy", String(page.busy));
				}
			}
		}
		const currentView = shell ? remoteItem(location.pathname)?.id : void 0;
		const ownName = document.getElementById("current-browser-name");
		for (const device of page.devices?.devices ?? []) {
			const row = rows.get(device.clientId);
			if (!row) continue;
			const destination = currentView === "settings" && device.thisBrowser && ownName ? ownName : devices;
			if (row.parentElement !== destination) destination.append(row);
			row.hidden = currentView === "settings" && !device.thisBrowser;
			for (const action of row.querySelectorAll("[data-action]")) action.hidden = currentView === "settings";
		}
		const ownMissing = document.getElementById("current-browser-name-missing");
		if (ownMissing) ownMissing.hidden = !!page.devices?.devices.some((device) => device.thisBrowser);
		renderOutcome();
		syncBusyMark();
	}
	async function run(action, owner = refresh) {
		if (otherHold()) return;
		cancelConfirmation();
		if (!busyMark.shown()) markOwner = owner;
		const pending = action();
		render();
		try {
			await pending;
		} catch (error) {
			page.notice = error instanceof Error ? error.message : "Action unavailable.";
		}
		render();
	}
	/** Paint management first; only the optional section changes when its read completes. */
	async function refreshView(cursor) {
		await page.refresh(cursor);
		render();
		if (page.access === "live") page.observeFirestore().then(renderService);
	}
	async function change(intent, owner) {
		if (otherHold()) return;
		await run(() => page.submit(intent), owner);
		if (page.outcome?.state === "committed" && !page.outcome.sessionEnded) await run(() => refreshView(), owner);
	}
	function submitButton(form) {
		return form.querySelector("button[type=submit]");
	}
	element("opening-form").addEventListener("submit", (event) => {
		event.preventDefault();
		if (!page.writable) return;
		if (!openingChanged()) {
			render();
			return;
		}
		change({
			kind: "setting",
			input: {
				operationId: crypto.randomUUID(),
				setting: "open",
				value: opening.value === "on"
			}
		}, submitButton(element("opening-form")));
	});
	element("limit-form").addEventListener("submit", (event) => {
		event.preventDefault();
		if (!page.writable) return;
		if (!limitChanged()) {
			render();
			return;
		}
		if (mode.value === "custom" && !custom.checkValidity()) return;
		const value = mode.value === "off" ? null : custom.value;
		change({
			kind: "setting",
			input: {
				operationId: crypto.randomUUID(),
				setting: "sessions-per-device",
				value
			}
		}, submitButton(element("limit-form")));
	});
	for (const field of [
		opening,
		mode,
		custom
	]) {
		field.addEventListener("input", render);
		field.addEventListener("change", render);
	}
	refresh.addEventListener("click", () => {
		if (page.busy) return;
		const ownedFocus = document.activeElement === refresh;
		let moved = false;
		const trackFocus = (event) => {
			if (event.target !== refresh && event.target !== document.body) moved = true;
		};
		const trackPointer = (event) => {
			if (!refresh.contains(event.target)) moved = true;
		};
		document.addEventListener("focusin", trackFocus);
		document.addEventListener("pointerdown", trackPointer);
		run(() => refreshView()).finally(() => {
			document.removeEventListener("focusin", trackFocus);
			document.removeEventListener("pointerdown", trackPointer);
			if (ownedFocus && !moved && refresh.isConnected && !refresh.disabled && (document.activeElement === document.body || document.activeElement === refresh)) refresh.focus({ preventScroll: true });
		});
	});
	function navigate(cursor, owner) {
		if (page.busy) return;
		for (const device of page.devices?.devices ?? []) {
			const name = rows.get(device.clientId)?.querySelector("input");
			if (name && name.value !== device.name) {
				page.notice = "Save or restore the unsent device name before changing pages.";
				render();
				name.focus();
				return;
			}
		}
		run(() => refreshView(cursor), owner);
	}
	element("copy-designation").addEventListener("click", () => {
		if (page.access !== "live") return;
		navigator.clipboard.writeText(`tmt remote members designate ${page.clientId}`).then(() => {
			element("designation-copy-feedback").textContent = "Copied.";
		}, () => {
			element("designation-copy-feedback").textContent = "Copy failed. Select the command and copy it manually.";
		});
	});
	element("remove-others").addEventListener("click", (event) => {
		if (event.detail > 1 || !page.writable) return;
		for (const device of page.devices?.devices ?? []) {
			const draft = rows.get(device.clientId)?.querySelector("input");
			if (draft && draft.value !== device.name) {
				page.notice = "Save or restore the unsent device name before changing pages.";
				render();
				draft.focus();
				return;
			}
		}
		if (page.removalPreview && page.removalPreview.count > 0) {
			const fingerprint = page.removalPreview.fingerprint;
			page.removalPreview = void 0;
			change({
				kind: "remove-others",
				input: {
					operationId: crypto.randomUUID(),
					fingerprint
				}
			}, removeOthers);
		} else {
			cancelConfirmation();
			run(() => page.previewRemoval(), removeOthers);
		}
	});
	element("cancel-remove-others").addEventListener("click", () => {
		page.removalPreview = void 0;
		render();
		element("remove-others").focus();
	});
	element("remove-others").addEventListener("keydown", (event) => {
		const key = event;
		if ((key.repeat || key.isComposing) && ["Enter", " "].includes(key.key)) event.preventDefault();
		if (key.key === "Escape") {
			page.removalPreview = void 0;
			render();
		}
	});
	more.addEventListener("click", () => navigate(page.devices?.nextCursor ?? null, more));
	first.addEventListener("click", () => navigate(null, first));
	for (const button of document.querySelectorAll("[data-recover]")) button.addEventListener("click", recoverOriginal);
	if (shell) window.addEventListener("popstate", () => {
		if (!page) return;
		render();
		if (remoteItem(location.pathname)?.id === "members" && page.access === "live" && peopleView && !people?.roster && !people?.busy && !ownerInvitations?.heldBy()) peopleView.refresh();
		enterInvites();
	}, { signal: shell?.signal });
	/** Invites reads once per entry or explicit refresh, after People confirms this browser is the
	* owner. An entry that meets another request in flight reads when that request settles. */
	async function enterInvites() {
		invitesUnread = remoteItem(location.pathname)?.id === "invites";
		await readInvites();
	}
	async function readInvites() {
		if (!invitesUnread || remoteItem(location.pathname)?.id !== "invites") return;
		if (!page || page.access !== "live") return;
		if (!people?.roster && !people?.busy && !otherHold()) await peopleView?.refresh();
		if (!mayInvite() || !invites || invites.busy || otherHold()) return;
		invitesUnread = false;
		await invitesView?.refresh();
	}
	inviteButton?.addEventListener("click", () => {
		if (!invitesView || !mayInvite()) return;
		if (remoteItem(location.pathname)?.id !== "invites") {
			shell?.navigate(new URL("/remote/invites", location.href).href);
			if (remoteItem(location.pathname)?.id !== "invites") return;
		}
		invitesView.invite();
	});
	async function initialize(session) {
		if (page) return;
		shell?.observedSession?.(session);
		invitationSession = session;
		const reopen = async () => {
			const next = await reopenSession(session, { retry: "bounded" });
			if (management(next).clientId !== page.clientId) throw new Error("The paired browser changed.");
			session = next;
			invitationSession = session;
			shell?.observedSession?.(session);
			if (invitationAttached) ownerInvitations?.(session);
			return session;
		};
		page = new ManagementPage(management(session), async () => management(await reopen()));
		const peopleRoot = shell && document.getElementById("people-content");
		if (shell && peopleRoot) {
			people = new MembersPage(members(session), async () => members(await reopen()), (error) => error instanceof RefusalError && [
				"REMOTE_GRANT_CHANGED",
				"REMOTE_SESSION_ENDED",
				"REMOTE_SESSION_EVICTED"
			].includes(error.code), accessRefused);
			peopleView = mountMembersPage(peopleRoot, people, shell.signal, () => page.access !== "live" || page.busy || page.outcome?.state === "unknown" || !!ownerInvitations?.heldBy(), () => {
				if (page) render();
			}, document.getElementById("people-observation"));
			const invitesRoot = document.getElementById("invites-content");
			if (invitesRoot) {
				invites = new IssuedInvitationsPage(invitations(session), async () => invitations(await reopen()));
				invitesView = mountInvitesPage(invitesRoot, invites, shell.signal, {
					otherBusy: () => !mayInvite() || page.busy || page.outcome?.state === "unknown" || !!people?.busy || people?.outcome?.state === "unknown" || !!ownerInvitations?.heldBy(),
					changed: () => {
						if (page) render();
					},
					confirm: () => {
						if (remoteItem(location.pathname)?.id !== "members") shell?.navigate(new URL("/remote/members", location.href).href);
						const section = document.getElementById("owner-invitation");
						section?.scrollIntoView({ block: "start" });
						(section?.querySelector("#invitation-candidate:not([hidden]) input") ?? section?.querySelector("#invitation-refresh"))?.focus();
					}
				});
			}
		}
		await run(() => refreshView());
		if (page.access === "live") {
			if (remoteItem(location.pathname)?.id === "members") await peopleView?.refresh();
			await enterInvites();
		}
	}
	function unavailable(error, report = true) {
		invitationAttached = false;
		ownerInvitations?.();
		guide?.observe("unconfirmed");
		const capacity = capacityCopy(error instanceof RefusalError ? error : void 0);
		const ended = error instanceof RefusalError && [
			"REMOTE_DEVICE_REVOKED",
			"REMOTE_GRANT_EXPIRED",
			"REMOTE_SESSION_ENDED",
			"REMOTE_SESSION_EVICTED"
		].includes(error.code);
		commandNotice(element("access"), capacity ?? (ended ? "Current browser access refused. Use the local CLI." : "Remote may have stopped, or its reply couldn't be verified. Your pairing is unchanged."));
		element("access-announcement").textContent = capacity ?? (ended ? "Access ended" : "Access unconfirmed");
		const verifiedEnded = error instanceof RefusalError && ["REMOTE_DEVICE_REVOKED", "REMOTE_GRANT_EXPIRED"].includes(error.code);
		if (report) registryState(verifiedEnded ? "Access ended" : "Unconfirmed");
		refresh.disabled = true;
		element("access-notice").dataset.tone = "blocked";
		element("controls-reason").textContent = capacity ?? "Current access is unavailable. Use the local CLI.";
	}
	try {
		if (shell && !initial) unavailable(void 0, false);
		else await initialize(initial ?? await reopenSession());
	} catch (error) {
		unavailable(error);
	}
	return (session) => {
		if (shell?.signal.aborted) return;
		if (page && management(session).clientId !== page.clientId) {
			registryState("Unconfirmed", void 0, null);
			return;
		}
		if (!page) initialize(session).catch(unavailable);
		else if (page.access === "live") {
			invitationSession = session;
			if (invitationAttached) ownerInvitations?.(session);
			else render();
		}
	};
}
//#endregion
export { settingsPage };
