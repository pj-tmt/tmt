import { ClientError, RefusalError, ReopenSessionError, budget, management, reopenSession } from "/sdk/remote-v1.js";
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
		id: "overview",
		path: "/remote/overview",
		label: "Overview",
		title: "Overview",
		page: "landing"
	},
	{
		id: "members",
		path: "/remote/members",
		label: "Members",
		title: "Members",
		page: "settings"
	},
	{
		id: "invites",
		path: "/remote/invites",
		label: "Invites",
		title: "Invites",
		page: "landing",
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
		id: "settings",
		path: "/remote/settings",
		label: "Settings",
		title: "Settings",
		page: "settings"
	},
	{
		id: "service",
		path: "/remote/service",
		label: "Service & upgrade",
		title: "Service & upgrade",
		page: "settings",
		adminOnly: true
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
	const href = (view) => new URL(REMOTE_ITEMS.find((item) => item.id === view).path, location.href).href;
	let projection;
	const run = /* @__PURE__ */ new Map();
	function paint() {
		run.clear();
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
		const remotes = [{
			...item("overview"),
			label: `This Remote · ${view?.text ?? "Checking"}`,
			current: showing !== "remotes"
		}];
		if (view?.action) {
			run.set("recover", () => actions.recover(view.action));
			remotes.push({
				id: "recover",
				label: view.action,
				href: "#",
				current: false,
				note: view.text
			});
		}
		shell.setMenu({
			title: "Remote",
			groups: [{ items: [item("remotes")] }, {
				label: "This machine",
				items: remotes
			}]
		}, (id) => {
			const action = run.get(id);
			if (!action) return false;
			action();
			return true;
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
		if (next.current.connection.word === "Connected" && next.address) shell.describe(next.address);
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
function iconActionTooltipPosition(anchor, tooltipWidth, tooltipHeight, viewport, gutter) {
	const maxWidth = Math.max(0, viewport.width - gutter * 2);
	const width = Math.min(tooltipWidth, maxWidth);
	const leftEdge = viewport.left + gutter;
	const belowTop = Math.max(viewport.top, anchor.bottom);
	const below = Math.max(0, viewport.top + viewport.height - gutter - belowTop);
	const above = Math.max(0, anchor.top - viewport.top - gutter);
	const flipped = tooltipHeight > below && above > below;
	const maxHeight = flipped ? above : below;
	const top = flipped ? anchor.top - Math.min(tooltipHeight, maxHeight) : belowTop;
	return {
		left: Math.max(leftEdge, Math.min((anchor.left + anchor.right - width) / 2, viewport.left + viewport.width - gutter - width)),
		top,
		maxWidth,
		maxHeight
	};
}
function placeIconActionTooltip(anchor, tooltip) {
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
		const next = iconActionTooltipPosition(box, tooltipBox.width, Math.max(tooltipBox.height, tooltip.scrollHeight), bounds, gutter);
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
function bindIconActionTooltip(wrapper, button, tooltip) {
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
		release = claimIconActionTooltip(document, dismiss);
		tooltip.showPopover();
		position = placeIconActionTooltip(button, tooltip);
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
			row.button.replaceChildren(row.label, ...item.checked ? [row.check] : []);
			row.button.disabled = item.disabled === true;
			row.button.setAttribute("role", item.role ?? (item.checked === void 0 ? "menuitem" : "menuitemradio"));
			if (item.checked === void 0) row.button.removeAttribute("aria-checked");
			else row.button.setAttribute("aria-checked", String(item.checked));
			if (item.separated) row.button.dataset.separated = "true";
			else delete row.button.dataset.separated;
			if (item.disabledReason) {
				row.reason.id = `${id}-reason-${index}`;
				row.reason.textContent = item.disabledReason;
				row.button.setAttribute("aria-describedby", row.reason.id);
				if (!row.reason.isConnected) row.root.append(row.reason);
			} else {
				row.reason.remove();
				row.button.removeAttribute("aria-describedby");
			}
			children.push(row.root);
		}
		for (const key of removed) {
			rows.get(key).root.remove();
			rows.delete(key);
		}
		for (const [index, child] of children.entries()) if (menu.childNodes[index] !== child) menu.insertBefore(child, menu.childNodes[index] ?? null);
		while (menu.childNodes.length > children.length) menu.lastChild.remove();
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
function node$1(tag, text = "") {
	const result = document.createElement(tag);
	result.textContent = text;
	return result;
}
function action(text, run, variant) {
	const button = node$1("button");
	const label = node$1("span", text);
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
	const wrapper = node$1("div");
	wrapper.className = browserUiClasses.field;
	const input = node$1("input");
	input.id = `remote-field-${++fieldCount}`;
	input.className = browserUiClasses.fieldControl;
	input.value = value;
	const text = node$1("label", label);
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
		renameCurrent: () => {},
		watch: () => () => {}
	});
	const listeners = /* @__PURE__ */ new Set();
	const fragment = location.hash;
	if (fragment) history.replaceState(null, "", location.pathname + location.search);
	const list = document.getElementById("remote-list");
	const status = document.getElementById("remote-status");
	const joinHost = document.getElementById("remote-join");
	const joinMenu = mountBrowserMenu(joinHost, {
		label: "Join a Remote",
		triggerText: "Join a Remote",
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
			else (shell ?? void 0)?.navigate(new URL(remotePath("overview"), location.href).href);
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
		const box = node$1("dialog");
		box.className = browserUiClasses.dialog;
		const header = node$1("header");
		header.className = browserUiClasses.dialogHeader;
		const heading = node$1("h2", title);
		heading.id = "remote-dialog-heading";
		header.append(heading);
		box.setAttribute("aria-labelledby", heading.id);
		const content = node$1("div");
		content.className = browserUiClasses.dialogBody;
		const body = node$1("div");
		const message = node$1("p");
		message.setAttribute("role", "status");
		content.append(body, message);
		const buttons = node$1("footer");
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
		view.body.append(node$1("p", "Only names and addresses are copied. No keys, pairings or drafts."));
		const choices = [];
		for (const row of plan.rows) {
			const item = node$1("section");
			item.className = browserUiClasses.section;
			const name = node$1("h3", row.entry.name);
			name.className = browserUiClasses.sectionHeading;
			name.dataset.variant = "title";
			item.append(name, node$1("p", `${kindLabel(row.entry)} · ${row.entry.url}`));
			if (row.status === "duplicate") {
				item.append(node$1("p", `Already added as ${row.existingName}`));
				choices.push(() => null);
			} else if (row.status === "collision") {
				item.append(node$1("p", `You already have a remote named ${row.entry.name}`));
				const alias = field("Remote name", row.suggestedName);
				const skip = node$1("input");
				skip.type = "checkbox";
				skip.className = browserUiClasses.checkbox;
				const skipLabel = node$1("label", "Skip");
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
		view.body.append(node$1("p", entry.kind === "firestore" ? new URL(entry.url).host : entry.url));
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
		view.body.append(node$1("p", "This only removes the entry here. It does not sign out or revoke access."));
		view.buttons.append(action("Forget", () => {
			save(() => remoteRegistry.remove(entry.name), view).then((saved) => {
				if (!saved) return;
				status.replaceChildren(node$1("span", `Forgot ${entry.name} in this browser. Access is unchanged. `), action("Undo", () => {
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
				const state = row.querySelector(".tmt-ui-list-state");
				state.textContent = `${entry.connection.mark} ${entry.connection.text}`;
				state.dataset.tone = entry.connection.tone;
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
			const row = node$1("li");
			row.className = browserUiClasses.listRow;
			row.dataset.remote = entry.id;
			row.dataset.thisMachine = String(entry.thisMachine);
			row.dataset.url = entry.url;
			row.dataset.kind = entry.reference?.kind ?? "local";
			row.dataset.leading = "";
			const leading = node$1("div");
			leading.className = browserUiClasses.listLeading;
			leading.append(createIcon(entry.backend === "Local" ? "terminal" : "box"));
			const title = node$1("div");
			title.className = browserUiClasses.listTitle;
			const label = node$1("span");
			label.className = `${browserUiClasses.listLabel} remote-label`;
			const open = node$1("a", entry.name);
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
				const box = node$1("span");
				box.className = browserUiClasses.chip;
				const value = node$1("span", text);
				value.className = browserUiClasses.chipLabel;
				box.append(value);
				return box;
			};
			if (entry.thisMachine && entry.name !== "This machine") label.append(chip("This machine"));
			const role = chip(entry.roleLabel);
			role.className += " remote-role";
			role.dataset.role = "";
			role.dataset.roleTone = entry.role === "owner" ? "selected" : entry.role === void 0 ? "muted" : "neutral";
			label.append(role);
			const description = node$1("span", `${entry.backend}${entry.project ? ` · ${entry.project}` : ""} · ${entry.url}`);
			description.className = browserUiClasses.listDescription;
			title.append(label, description);
			const meta = node$1("div");
			meta.className = browserUiClasses.listMeta;
			const state = node$1("div", `${entry.connection.mark} ${entry.connection.text}`);
			state.className = browserUiClasses.listState;
			state.dataset.tone = entry.connection.tone;
			meta.append(state);
			const actions = node$1("div");
			actions.className = browserUiClasses.listActions;
			if (entry.reference) {
				const reference = entry.reference;
				const destructive = node$1("span", "Forget");
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
			row.append(leading, title, meta, actions);
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
		if (!value || ["#remotes", "#add-remote"].includes(value)) return;
		history.replaceState(null, "", location.pathname + location.search);
		receive(value);
	}, { signal: (shell ?? void 0)?.signal });
	const observe = (state, checkedAddress, role) => {
		observed = state;
		if (state !== "Connected" || role === null) observedRole = void 0;
		else if (role !== void 0) observedRole = role;
		if (checkedAddress) address = checkedAddress;
		paint();
	};
	return Object.assign(observe, {
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
			heldBy = reason;
		}
	});
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
/** Copy-only references; the browser never submits these commands. */
var serviceCommands = {
	status: "tmt remote status --layers",
	preview: "tmt remote deploy firestore --project='<project-id>' --region='<region>' --defaults --json"
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
function node(tag, text = "") {
	const element = document.createElement(tag);
	element.textContent = text;
	return element;
}
function mark(state) {
	const [icon, text, tone] = states[state];
	const label = node("span");
	label.dataset.tone = tone;
	label.className = browserUiClasses.listState;
	const symbol = node("span", icon);
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
	const anchor = node("a", text);
	anchor.href = href;
	anchor.target = "_blank";
	anchor.rel = "noopener noreferrer";
	return anchor;
}
function checks() {
	const section = node("section");
	section.className = "firestore-layer";
	section.setAttribute("aria-label", "Checks before an update");
	section.append(node("h3", "Before an update"));
	const rows = [
		{
			id: "version",
			title: "Deployed version and required version",
			detail: "Version comparison is not available here. Check this Remote’s setup in this machine’s terminal before upgrading.",
			command: serviceCommands.status
		},
		{
			id: "changes",
			title: "What will change",
			detail: "No deployment plan has been checked here. Preview the exact changes in this machine’s terminal.",
			command: serviceCommands.preview
		},
		{
			id: "permissions",
			title: "Account permissions",
			detail: "Permissions have not been checked here. Use the Firebase account that manages the project in this machine’s terminal to check them.",
			command: serviceCommands.preview
		}
	];
	for (const row of rows) {
		const item = node("section");
		item.dataset.serviceCheck = row.id;
		const title = node("h4", `${row.title}: `);
		title.append(mark("unknown"));
		item.append(title, node("p", row.detail), commandCopy(row.command));
		section.append(item);
	}
	section.append(node("p", "Replace <project-id> and <region> with your Firebase project and its region. The preview command does not authorize deployment."));
	return section;
}
function guidance() {
	const section = node("section");
	section.className = "firestore-layer";
	section.append(node("h3", "Where to check and make changes"));
	const steps = node("ol");
	const consoleStep = node("li", "In the provider console, choose your Firebase project. Review its sign-in, database rules and usage. ");
	consoleStep.append(link("Open Firebase console", "https://console.firebase.google.com/"));
	const permissionStep = node("li", "In the provider console, open Project settings → Users and permissions. Ask the project administrator for missing access. ");
	permissionStep.append(link("Firebase roles and permissions", "https://firebase.google.com/docs/projects/iam/roles"));
	steps.append(node("li", "In this machine’s terminal, install the Firebase CLI and sign in with the account that manages the project."), consoleStep, node("li", "In this machine’s terminal, copy the preview command above to review changes and check permissions. Review and authorize any deployment there."), permissionStep, node("li", "After you act in the terminal or provider console, return here and use Refresh values. That rechecks this machine’s saved setup facts; it does not run a live provider check."));
	section.append(steps);
	return section;
}
function paidStorage() {
	const paid = node("p", "Paid Firebase Blaze usage and separate consent are required when file storage becomes available. ");
	paid.append(link("Firebase pricing plans", "https://firebase.google.com/docs/projects/billing/firebase-pricing-plans"));
	return paid;
}
function futureServices(includeStorage = false) {
	const section = node("section");
	section.className = "firestore-layer";
	section.setAttribute("aria-label", "Other services");
	section.append(node("h3", "Other services"));
	if (includeStorage) section.append(node("p", "File storage (paid add-on) — Not available in this release."), paidStorage());
	section.append(node("p", "Cloudflare — Coming soon."));
	return section;
}
var painted = /* @__PURE__ */ new WeakMap();
function renderFirestore(target, access, view) {
	const projection = JSON.stringify([access, view]);
	if (painted.get(target) === projection) return;
	painted.set(target, projection);
	const announcement = document.getElementById("firestore-announcement");
	const summary = access === "checking" ? "" : access !== "confirmed" || !view ? "Service status could not be confirmed." : !view.firestoreLayers.length ? "Firestore is not configured." : view.firestoreLayers.map((layer) => `${firestoreTitles[layer.layer]}: ${layer.prerequisites.some((item) => item.item === "support" && item.reason === "not-implemented") ? "Not available in this release" : states[layer.state][1]}.`).join(" ");
	if (summary && announcement.textContent !== summary) announcement.textContent = summary;
	target.replaceChildren();
	if (access !== "confirmed" || !view) {
		target.append(node("p", access === "checking" ? "Checking service status…" : "Service status could not be confirmed."));
		if (access !== "checking") target.append(checks(), guidance(), futureServices(true));
		target.dataset.tone = access === "checking" ? "waiting" : "blocked";
		return;
	}
	target.dataset.tone = "review";
	target.append(node("p", view.firestoreLayers.length ? "Cloud service: Firebase (Firestore)." : "Firestore is not configured."));
	target.append(node("p", "These setup facts come from this machine. They are not a live check of your Firebase project."));
	for (const layer of view.firestoreLayers) {
		const group = node("section");
		group.className = "firestore-layer";
		group.dataset.layer = layer.layer;
		const title = firestoreTitles[layer.layer];
		if (layer.prerequisites.some((p) => p.item === "support" && p.reason === "not-implemented")) {
			const line = node("h3", `${title} — Not available in this release.`);
			group.append(line);
			if (layer.layer === "attachments") group.append(paidStorage());
		} else {
			const heading = node("h3", `${title} `);
			heading.append(mark(layer.state));
			group.append(heading);
			const list = node("ul");
			for (const item of layer.prerequisites) {
				const row = node("li");
				row.dataset.item = item.item;
				const displayState = item.item === "indexes" && item.state !== "enabled" ? item.reason === "building" ? "building" : "unknown" : item.state;
				row.append(node("span", `${firestoreItems[item.item]}: `), mark(displayState));
				if (item.reason) row.append(node("p", firestoreReasons[item.item][item.reason][0]));
				if (item.next) {
					const nextLine = firestoreReasons[item.item][item.reason][2];
					const hint = node("p", nextLine ?? "Run this in this machine’s terminal:");
					row.append(hint, commandCopy(item.next));
				}
				list.append(row);
			}
			group.append(list);
			if (layer.layer === "sharing" && layer.state === "enabled") group.append(node("p", "Each client stops itself before the free-plan limits. TMT does not check the project's remaining quota, so Firebase may still refuse requests."));
		}
		target.append(group);
	}
	target.append(checks(), guidance(), futureServices(!view.firestoreLayers.some((layer) => layer.layer === "attachments")));
	const details = node("details");
	details.id = "firestore-budget";
	details.className = browserUiClasses.disclosure;
	details.append(node("summary", "Firestore free-plan limits"));
	const published = view.firestoreBudget;
	details.append(node("h3", `Published limits as of ${published.readOn}`));
	const table = node("table");
	table.className = browserUiClasses.table;
	const head = node("tr");
	for (const text of ["Limit", "Published allowance"]) {
		const cell = node("th", text);
		cell.scope = "col";
		head.append(cell);
	}
	const thead = node("thead");
	thead.append(head);
	table.append(thead);
	const body = node("tbody");
	for (const [key, label, per, bytes] of budget.publishedLimitRows) {
		const row = node("tr");
		const title = node("th", label);
		title.scope = "row";
		row.append(title, node("td", allowance(published.limits[key], bytes, per)));
		body.append(row);
	}
	table.append(body);
	details.append(table);
	details.append(node("p", "Google may change these limits. They are shared by the whole Firebase project. Daily limits reset around midnight Pacific time."));
	details.append(node("p", "Remote can't see your actual usage; check it in the Firebase console."));
	details.append(node("p", `Each device warns at ${published.guard.warnPercent}% and stops at ${published.guard.refusePercent}% of its share of the daily limits.`));
	target.append(details);
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
		} else if (this.outcome?.state === "unknown") this.notice = this.intent?.kind === "revoke" || this.intent?.kind === "remove-others" ? "Could not confirm the removal. Check its result before trying again." : this.freshAttempted ? "The saved value is still unconfirmed. Check with tmt remote settings or tmt remote devices." : "This change could not be confirmed. Check the saved value before making another change.";
		else if (this.outcome?.state === "refused") this.notice = this.outcome.reason === "REMOTE_DEVICE_LIST_CHANGED" ? "The device list changed. Review it, then confirm again." : this.outcome.reason === "REMOTE_MANAGEMENT_CAPACITY" ? "Browser change limit reached. Use the local CLI; do not retry or reset storage." : this.outcome.reason === "REMOTE_MANAGEMENT_READ_ONLY" ? "This browser is read-only. Use the local CLI to make changes." : "Change refused. Check the current values or use the local CLI.";
	}
};
function accessRefused(error) {
	if (error instanceof ReopenSessionError) return error.reason === "revoked" || error.reason === "expired";
	return error instanceof RefusalError && ["REMOTE_DEVICE_REVOKED", "REMOTE_GRANT_EXPIRED"].includes(error.code);
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
	const registryState = registry ?? remoteRegistryPage(void 0, shell);
	registryState.blockedBy(() => {
		if (!page) return "";
		if (page.busy) return "A request is in progress. Wait for its outcome.";
		if (page.outcome?.state === "unknown") return "Another change is still unconfirmed. Check the saved value first.";
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
		button.disabled = unavailable;
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
		for (const button of document.querySelectorAll("button.tmt-ui-action")) if (!button.closest("#remotes, #remote-head, #remote-switcher, #remote-context, #remote-local-name, #remote-locked")) markSlot(button);
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
		registryState(page.access === "live" ? "Connected" : page.access === "lost" ? "Access ended" : page.access === "checking" ? "Checking" : "Unconfirmed", void 0, managementRole(page.access, page.settings?.capabilities.settingsWrite) ?? null);
		if (confirmation) {
			const target = page.devices?.devices.find((device) => device.clientId === confirmation.clientId);
			if (!page.writable || !target || targetSignature(target) !== confirmation.signature) cancelConfirmation();
		}
		renderFirestore(element("firestore-content"), page.firestoreAccess, page.firestore);
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
		if (!designation.hidden) element("designation-code").textContent = `tmt remote devices designate ${page.clientId}`;
		element("access-notice").dataset.tone = page.access === "live" ? editable ? "working" : "review" : page.access === "checking" ? "waiting" : "blocked";
		const reason = capacity ?? (!editable ? page.access === "live" ? "" : "Current browser access could not be confirmed." : page.outcome?.state === "unknown" ? "Another change is still unconfirmed. Check the saved value first." : page.outcome?.state === "refused" && page.outcome.reason === "REMOTE_MANAGEMENT_CAPACITY" ? "Browser change limit reached. Use the local CLI; do not retry or reset storage." : "");
		element("controls-reason").textContent = reason;
		element("controls-reason").hidden = !reason;
		for (const button of document.querySelectorAll("button")) {
			if (button.closest("#remotes, #remote-head, #remote-switcher, #remote-context, #remote-local-name, #remote-locked")) continue;
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
					row.className = "device";
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
			row.hidden = (currentView === "my-devices" || currentView === "settings") && !device.thisBrowser;
			for (const action of row.querySelectorAll("[data-action]")) action.hidden = currentView === "settings";
		}
		const ownMissing = document.getElementById("current-browser-name-missing");
		if (ownMissing) ownMissing.hidden = !!page.devices?.devices.some((device) => device.thisBrowser);
		renderOutcome();
		syncBusyMark();
	}
	async function run(action, owner = refresh) {
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
		if (page.access === "live") page.observeFirestore().then(() => renderFirestore(element("firestore-content"), page.firestoreAccess, page.firestore));
	}
	async function change(intent, owner) {
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
		navigator.clipboard.writeText(`tmt remote devices designate ${page.clientId}`).then(() => {
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
		if (page) render();
	}, { signal: shell?.signal });
	async function initialize(session) {
		if (page) return;
		page = new ManagementPage(management(session), async () => {
			session = await reopenSession(session, { retry: "bounded" });
			return management(session);
		});
		await run(() => refreshView());
	}
	function unavailable(error, report = true) {
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
	};
}
//#endregion
export { settingsPage };
