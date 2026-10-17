import { ClientError, RefusalError, ReopenSessionError, budget, management, reopenSession } from "/sdk/remote-v1.js";
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
//#region src/remote-state.ts
/** Current-origin observation only. A foreign entry has no mark or live-access claim. */
function remoteState(state, current = true) {
	if (!current) return {
		text: "not checked",
		word: "not checked",
		mark: "",
		tone: "muted",
		action: ""
	};
	const [text, word, mark, tone, action] = state === "Connected" ? [
		"Connected",
		"connected",
		"●",
		"working",
		""
	] : state === "Checking" ? [
		"Checking",
		"connecting",
		"◌",
		"waiting",
		""
	] : state === "Access ended" ? [
		"Access ended",
		"access ended",
		"✗",
		"blocked",
		"Pair again"
	] : state === "Different machine" ? [
		"Different machine",
		"different machine",
		"△",
		"waiting",
		"Pair again"
	] : state === "Not paired" || state === "Pairing unreadable" ? [
		state,
		"not paired",
		"○",
		"waiting",
		"Pair again"
	] : [
		"Unconfirmed",
		"unreachable",
		"○",
		"blocked",
		"Retry"
	];
	return {
		text: `${mark} ${text}`,
		word,
		mark,
		tone,
		action
	};
}
/** Both page owners and the menu identify the current Local row by the same door root. */
function currentRemote(entry, address, pageUrl) {
	return entry.kind === "local" && (entry.url === address || new URL(entry.url).href === new URL("./", pageUrl).href);
}
//#endregion
//#region src/remote-menu.ts
function remoteMenu(actions) {
	const host = document.getElementById("remote-switcher");
	if (!host) return () => {};
	const trigger = document.createElement("button");
	trigger.type = "button";
	trigger.className = "tmt-ui-menu-trigger remote-chip";
	trigger.setAttribute("aria-haspopup", "menu");
	trigger.setAttribute("aria-controls", "remote-menu");
	trigger.setAttribute("aria-expanded", "false");
	const surface = document.createElement("div");
	surface.id = "remote-menu";
	surface.popover = "auto";
	surface.className = "tmt-ui-menu-surface remote-menu";
	surface.setAttribute("role", "menu");
	surface.setAttribute("aria-label", "Remotes");
	const status = document.createElement("p");
	status.className = "tmt-ui-menu-reason";
	host.append(trigger, surface);
	let catalog = {
		version: 1,
		entries: []
	};
	let state = "Checking";
	let address = "";
	let search = "";
	let searchAt = 0;
	const buttons = /* @__PURE__ */ new Map();
	const selected = () => catalog.entries.find((entry) => currentRemote(entry, address, location.href));
	const enabled = () => [...surface.querySelectorAll("button:not(:disabled)")];
	function close(refocus = false) {
		if (surface.matches(":popover-open")) surface.hidePopover();
		if (refocus) trigger.focus({ preventScroll: true });
	}
	function item(key, label, run) {
		let button = buttons.get(key);
		if (!button) {
			button = document.createElement("button");
			button.type = "button";
			button.className = "tmt-ui-menu-item";
			button.tabIndex = -1;
			button.dataset.key = key;
			buttons.set(key, button);
		}
		button.onclick = () => {
			close();
			run();
		};
		button.dataset.search = label;
		button.textContent = label;
		return button;
	}
	function paint() {
		const view = remoteState(state);
		const alias = selected()?.name ?? "this machine";
		trigger.textContent = `${view.mark} ${alias} ▾`;
		trigger.title = `Remote: ${alias}, ${view.word}`;
		trigger.setAttribute("aria-label", trigger.title);
		trigger.dataset.tone = view.tone;
		const entries = [...catalog.entries].sort((a, b) => Number(a.kind === "firestore") - Number(b.kind === "firestore"));
		const keep = /* @__PURE__ */ new Set();
		const reason = actions.blocked();
		const append = (key, label, run, checked, disabled) => {
			const button = item(key, label, run);
			button.dataset.search = key.startsWith("remote:") ? key.slice(7) : "this machine";
			button.setAttribute("role", "menuitemradio");
			button.setAttribute("aria-checked", String(checked));
			if (checked) {
				const check = document.createElement("span");
				check.textContent = "✓";
				check.setAttribute("aria-hidden", "true");
				button.append(check);
			}
			button.disabled = disabled;
			button.dataset.tone = checked ? view.tone : "muted";
			button.setAttribute("aria-describedby", "remote-menu-reason");
			keep.add(key);
			return button;
		};
		const rows = [];
		if (!selected()) rows.push(append("current", `${view.mark} this machine · ${view.word}`, () => {}, true, !!view.action));
		for (const entry of entries) {
			const here = currentRemote(entry, address, location.href), shown = remoteState(state, here);
			rows.push(append(`remote:${entry.name}`, here ? `${shown.mark} ${entry.name} · ${shown.word}` : `Open ${entry.name} · ${entry.kind === "local" ? "Local" : `Firebase · ${entry.project}`} · not checked`, () => {
				if (!here) actions.open(entry);
			}, here, here ? !!shown.action : !!reason));
		}
		if (view.action) {
			const key = "recover";
			keep.add(key);
			const button = item(key, view.action, () => actions.recover(remoteState(state).action));
			button.setAttribute("role", "menuitem");
			rows.push(button);
		}
		for (const [key, label, run] of [[
			"add",
			"Add remote…",
			actions.add
		], [
			"manage",
			"Manage remotes",
			actions.manage
		]]) {
			keep.add(key);
			const button = item(key, label, run);
			button.setAttribute("role", "menuitem");
			rows.push(button);
		}
		status.id = "remote-menu-reason";
		status.textContent = reason || (view.action ? view.text : "");
		status.hidden = !status.textContent;
		for (const [key, button] of buttons) if (!keep.has(key)) {
			button.remove();
			buttons.delete(key);
		}
		rows.forEach((button, index) => {
			if (surface.children[index] !== button) surface.insertBefore(button, surface.children[index] ?? null);
		});
		surface.append(status);
		const focused = enabled().find((button) => button === document.activeElement) ?? enabled()[0];
		for (const button of buttons.values()) button.tabIndex = button === focused ? 0 : -1;
	}
	function place() {
		const anchor = trigger.getBoundingClientRect();
		const box = surface.getBoundingClientRect();
		const width = document.documentElement.clientWidth;
		surface.style.left = `${Math.max(8, Math.min(anchor.left, width - box.width - 8))}px`;
		const below = innerHeight - anchor.bottom - 8;
		const above = anchor.top - 8;
		const up = box.height > below && above > below;
		surface.style.maxHeight = `${Math.max(0, up ? above : below)}px`;
		surface.style.top = `${up ? Math.max(8, anchor.top - Math.min(box.height, above)) : anchor.bottom}px`;
	}
	async function open(end = false) {
		await actions.refresh();
		if (document.activeElement !== trigger) return;
		paint();
		if (!surface.matches(":popover-open")) surface.showPopover();
		place();
		search = "";
		(end ? enabled().at(-1) : enabled()[0])?.focus();
	}
	let openAtPointerdown = false;
	trigger.addEventListener("pointerdown", () => {
		openAtPointerdown = surface.matches(":popover-open");
	});
	trigger.addEventListener("click", (event) => {
		const dismiss = event.detail > 0 && openAtPointerdown;
		openAtPointerdown = false;
		if (dismiss || surface.matches(":popover-open")) close();
		else {
			trigger.focus({ preventScroll: true });
			open();
		}
	});
	trigger.addEventListener("keydown", (event) => {
		if (event.key === "ArrowDown" || event.key === "ArrowUp") {
			event.preventDefault();
			open(event.key === "ArrowUp");
		}
	});
	surface.addEventListener("toggle", () => {
		trigger.setAttribute("aria-expanded", String(surface.matches(":popover-open")));
		if (surface.matches(":popover-open")) place();
	});
	surface.addEventListener("focusin", () => {
		for (const button of buttons.values()) button.tabIndex = button === document.activeElement ? 0 : -1;
	});
	surface.addEventListener("keydown", (event) => {
		if (event.key === "Escape") {
			event.preventDefault();
			close(true);
			return;
		}
		if (event.key === "Tab") {
			close();
			return;
		}
		if (event.isComposing || event.altKey || event.ctrlKey || event.metaKey) return;
		const items = enabled(), at = items.findIndex((item) => item === document.activeElement);
		let next;
		if (event.key === "ArrowDown") next = items[(at + 1) % items.length];
		else if (event.key === "ArrowUp") next = items[(at - 1 + items.length) % items.length];
		else if (event.key === "Home") next = items[0];
		else if (event.key === "End") next = items.at(-1);
		else if (event.key.length === 1 && event.key !== " ") {
			const now = performance.now();
			search = (now - searchAt > 700 ? "" : search) + event.key.toLowerCase();
			searchAt = now;
			next = [...items.slice(at + 1), ...items.slice(0, at + 1)].find((item) => (item.dataset.search ?? "").toLowerCase().startsWith(search));
		} else return;
		event.preventDefault();
		next?.focus();
	});
	window.addEventListener("resize", () => {
		if (surface.matches(":popover-open")) place();
	});
	window.addEventListener("scroll", () => {
		if (surface.matches(":popover-open")) place();
	}, true);
	return (next, observed, checkedAddress) => {
		catalog = next;
		state = observed;
		address = checkedAddress;
		paint();
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
		const r = object(JSON.parse(text), [
			"version",
			"kind",
			"publicWebConfig",
			"checksum"
		]);
		valid$1(r.version === 1 && r.kind === "firestore" && typeof r.checksum === "string" && /^[a-f0-9]{64}$/.test(r.checksum));
		const publicConfig = parsePublicWebConfig(r.publicWebConfig);
		valid$1(origin === `https://${publicConfig.projectId}.web.app`);
		valid$1(await checksum({
			kind: r.kind,
			publicWebConfig: publicConfig,
			version: r.version
		}) === r.checksum && text === canonical(r));
		valid$1(link === `${origin}/pair#${base64url(bytes)}`);
		return publicConfig;
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
	if (url.protocol === "https:" && url.hostname.endsWith(".web.app") && url.pathname === "/pair") return {
		name: alias,
		kind: "firestore",
		url: address,
		project: (await verifyHostedLink(address, url.origin).catch(() => {
			valid(false);
		})).projectId
	};
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
	return `${checked.url}#${params}`;
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
function action(text, run) {
	const button = node$1("button", text);
	button.type = "button";
	button.className = "tmt-ui-action";
	button.addEventListener("click", run);
	return button;
}
function field(label, value) {
	const wrapper = node$1("label", label);
	wrapper.className = "tmt-ui-field-label";
	const input = node$1("input");
	input.className = "tmt-ui-field-control";
	input.value = value;
	wrapper.append(input);
	return {
		label: wrapper,
		input
	};
}
function kindLabel(entry) {
	return entry.kind === "local" ? "Local" : "Firebase";
}
function remoteRegistryPage(blocked = () => "") {
	const root = document.getElementById("remotes");
	if (!root) return () => {};
	const fragment = location.hash;
	if (fragment) history.replaceState(null, "", location.pathname + location.search);
	const list = document.getElementById("remote-list");
	const status = document.getElementById("remote-status");
	const add = document.getElementById("remote-add");
	let observed = "Checking";
	let address = document.documentElement.dataset.tmtPage === "settings" ? new URL("./", location.href).href : void 0;
	let currentDialog;
	let catalog = {
		version: 1,
		entries: []
	};
	const settings = new URL("settings", new URL("./", location.href));
	const menu = remoteMenu({
		refresh: () => render(),
		open: (entry) => {
			if (blocked()) return;
			remoteRegistry.navigation(entry).then((url) => location.assign(url)).catch((error) => showError(error, status));
		},
		add: () => {
			if (document.documentElement.dataset.tmtPage === "settings") add.click();
			else location.assign(`${settings.href}#add-remote`);
		},
		manage: () => {
			if (document.documentElement.dataset.tmtPage === "settings") {
				root.scrollIntoView();
				add.focus({ preventScroll: true });
			} else location.assign(`${settings.href}#remotes`);
		},
		recover: (action) => {
			if (action === "Retry") {
				const retry = document.getElementById("check") ?? document.getElementById("refresh");
				if (retry && !retry.disabled) retry.click();
				else location.assign(new URL("./", location.href).href);
			} else if (document.documentElement.dataset.tmtPage === "settings") location.assign(new URL("./", location.href).href);
			else {
				const pair = [
					"copy-refused-pair",
					"copy-missing-pair",
					"copy-unreadable-pair",
					"copy-different-pair"
				].map((id) => document.getElementById(id)).find((button) => button && button.getClientRects().length > 0);
				pair?.scrollIntoView();
				pair?.focus();
			}
		},
		blocked
	});
	const showError = (error, slot) => {
		slot.textContent = error instanceof RemoteRegistryError ? error.message : "The remotes could not be read";
	};
	const dialog = (title, opener) => {
		currentDialog?.close();
		const box = node$1("dialog");
		box.className = "remote-dialog";
		const heading = node$1("h2", title);
		heading.id = "remote-dialog-heading";
		box.setAttribute("aria-labelledby", heading.id);
		const body = node$1("div");
		const message = node$1("p");
		message.setAttribute("role", "status");
		const buttons = node$1("div");
		buttons.className = "remote-actions";
		box.append(heading, body, message, buttons);
		root.append(box);
		const end = () => box.close();
		box.addEventListener("close", () => {
			box.remove();
			if (currentDialog === box) {
				currentDialog = void 0;
				(opener.isConnected ? opener : add).focus();
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
		} catch (error) {
			showError(error, view.message);
		} finally {
			buttons.forEach((b) => {
				b.disabled = false;
			});
		}
	};
	const importView = (plan) => {
		const view = dialog("Add these remotes to this browser?", add);
		view.body.append(node$1("p", "Only names and addresses are copied. No keys, pairings or drafts."));
		const choices = [];
		for (const row of plan.rows) {
			const item = node$1("section");
			item.className = "remote-import-row";
			item.append(node$1("h3", row.entry.name), node$1("p", `${kindLabel(row.entry)} · ${row.entry.url}`));
			if (row.status === "duplicate") {
				item.append(node$1("p", `Already added as ${row.existingName}`));
				choices.push(() => null);
			} else if (row.status === "collision") {
				item.append(node$1("p", `You already have a remote named ${row.entry.name}`));
				const alias = field("Remote name", row.suggestedName);
				const skip = node$1("input");
				skip.type = "checkbox";
				const skipLabel = node$1("label", "Skip");
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
		view.body.append(node$1("p", entry.kind === "firestore" ? `${entry.project}.web.app` : entry.url));
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
		const view = dialog(`Remove ${entry.name} from this browser?`, opener);
		view.body.append(node$1("p", "This only removes the entry here. It does not sign out or revoke access."));
		view.buttons.append(action("Remove", () => {
			save(() => remoteRegistry.remove(entry.name), view);
		}), action("Cancel", view.end));
		view.box.showModal();
	};
	const current = (entry) => currentRemote(entry, address, location.href);
	const render = async () => {
		try {
			catalog = await remoteRegistry.list();
			const rows = catalog.entries.map((entry) => {
				const row = node$1("li");
				row.dataset.remote = entry.name;
				row.dataset.url = entry.url;
				row.dataset.kind = entry.kind;
				row.append(node$1("h3", entry.name), node$1("p", `${kindLabel(entry)} · ${entry.url}`));
				const state = node$1("p", remoteState(observed, current(entry)).text);
				state.className = "remote-state";
				state.dataset.tone = remoteState(observed, current(entry)).tone;
				row.append(state);
				const actions = node$1("div");
				actions.className = "remote-actions";
				const open = action("Open", () => {
					(async () => {
						try {
							if (!blocked()) location.assign(await remoteRegistry.navigation(entry));
						} catch (error) {
							showError(error, status);
						}
					})();
				});
				open.dataset.action = "open";
				const renameButton = action("Rename", () => rename(entry, renameButton));
				const removeButton = action("Remove", () => remove(entry, removeButton));
				renameButton.setAttribute("aria-label", `Rename ${entry.name}`);
				removeButton.setAttribute("aria-label", `Remove ${entry.name}`);
				if (!current(entry)) {
					open.disabled = !!blocked();
					open.title = blocked();
					actions.append(open);
				}
				actions.append(renameButton, removeButton);
				row.append(actions);
				return row;
			});
			list.replaceChildren(...rows);
			if (!rows.length) list.append(node$1("li", "No remotes added in this browser."));
			menu(catalog, observed, address ?? "");
		} catch (error) {
			showError(error, status);
		}
	};
	add.addEventListener("click", () => {
		const view = dialog("Add remote", add);
		const alias = field("Remote name", "");
		const url = field("Door address or remote link", "");
		view.body.append(alias.label, url.label);
		view.buttons.append(action("Continue", () => {
			reference(alias.input.value, url.input.value).then((entry) => {
				view.end();
				confirmAdd(entry, add);
			}).catch((error) => showError(error, view.message));
		}), action("Cancel", view.end));
		view.box.showModal();
	});
	render().then(() => {
		if (fragment === "#add-remote") add.click();
		else if (fragment === "#remotes") {
			root.scrollIntoView();
			add.focus({ preventScroll: true });
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
	});
	return (state, checkedAddress) => {
		observed = state;
		if (checkedAddress) address = checkedAddress;
		for (const row of list.querySelectorAll("[data-remote]")) {
			const stateNode = row.querySelector(".remote-state");
			const entry = catalog.entries.find((entry) => entry.name === row.dataset.remote);
			const isCurrent = !!entry && currentRemote(entry, address, location.href);
			const view = remoteState(observed, isCurrent);
			stateNode.textContent = view.text;
			stateNode.dataset.tone = view.tone;
			const open = row.querySelector("[data-action=\"open\"]");
			if (open) {
				open.disabled = !!blocked();
				open.title = blocked();
			}
			if (isCurrent) row.querySelector("[data-action=\"open\"]")?.remove();
		}
		menu(catalog, observed, address ?? "");
	};
}
//#endregion
//#region src/firestore.ts
var firestoreTitles = {
	sharing: "Page sharing",
	operations: "Operation sync",
	attachments: "Attachment storage"
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
function command(text) {
	const code = node("code", text);
	code.className = "tmt-ui-code";
	return code;
}
function mark(state) {
	const [icon, text, tone] = states[state];
	const label = node("span");
	label.dataset.tone = tone;
	label.className = "firestore-state";
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
var painted = /* @__PURE__ */ new WeakMap();
function renderFirestore(target, access, view) {
	const projection = JSON.stringify([access, view]);
	if (painted.get(target) === projection) return;
	painted.set(target, projection);
	const announcement = document.getElementById("firestore-announcement");
	const summary = access === "checking" ? "" : access !== "confirmed" || !view ? "Firestore setup could not be confirmed." : !view.firestoreLayers.length ? "Firestore is not configured." : view.firestoreLayers.map((layer) => `${firestoreTitles[layer.layer]}: ${layer.prerequisites.some((item) => item.item === "support" && item.reason === "not-implemented") ? "Not available in this release" : states[layer.state][1]}.`).join(" ");
	if (summary && announcement.textContent !== summary) announcement.textContent = summary;
	target.replaceChildren();
	if (access !== "confirmed" || !view) {
		target.append(node("p", access === "checking" ? "Checking recorded Firestore setup…" : "Firestore setup could not be confirmed."));
		if (access !== "checking") {
			const hint = node("p", "Check with ");
			hint.append(command("tmt remote status --layers"), document.createTextNode(" on the machine that runs Remote."));
			target.append(hint);
		}
		target.dataset.tone = access === "checking" ? "waiting" : "blocked";
		return;
	}
	target.dataset.tone = "review";
	if (!view.firestoreLayers.length) target.append(node("p", "Firestore is not configured."));
	for (const layer of view.firestoreLayers) {
		const group = node("section");
		group.className = "firestore-layer";
		group.dataset.layer = layer.layer;
		const title = firestoreTitles[layer.layer];
		if (layer.prerequisites.some((p) => p.item === "support" && p.reason === "not-implemented")) {
			const line = node("h3", `${title} — Not available in this release.`);
			group.append(line);
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
					const hint = node("p", nextLine ?? "Next command on the machine that runs Remote: ");
					if (!nextLine) hint.append(command(item.next));
					row.append(hint);
				}
				list.append(row);
			}
			group.append(list);
		}
		target.append(group);
	}
	const details = node("details");
	details.id = "firestore-budget";
	details.append(node("summary", "Firestore free-plan limits"));
	const published = view.firestoreBudget;
	details.append(node("h3", `Published limits as of ${published.readOn}`));
	const table = node("table");
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
		return this.access === "live" && this.settings?.capabilities.settingsWrite === true && !this.busy && this.outcome?.state !== "unknown" && !(this.outcome?.state === "refused" && this.outcome.reason === "REMOTE_MANAGEMENT_CAPACITY");
	}
	async refresh(cursor = this.cursor) {
		if (this.busy || this.access === "lost") return;
		this.busy = true;
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
	async submit(intent) {
		if (!this.writable) throw new Error("Management controls are read-only.");
		this.intent = Object.freeze({
			...intent,
			input: Object.freeze({ ...intent.input })
		});
		this.outcome = void 0;
		this.freshAttempted = false;
		this.busy = true;
		this.notice = intent.kind === "revoke" ? "Revoking device…" : intent.kind === "talk" ? intent.input.enabled ? "Enabling sending…" : "Disabling sending…" : "Saving…";
		try {
			const frozen = this.intent;
			this.outcome = frozen.kind === "setting" ? await this.client.set(frozen.input) : frozen.kind === "rename" ? await this.client.rename(frozen.input) : frozen.kind === "talk" ? await this.client.talk(frozen.input) : await this.client.revoke(frozen.input);
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
			this.notice = intent?.kind === "setting" ? intent.input.setting === "open" ? "Browser opening saved." : "Session limit saved. Applies to new sessions." : intent?.kind === "rename" ? "Device renamed." : intent?.kind === "talk" ? intent.input.enabled ? "Sending enabled for this device." : "Sending disabled for this device." : "Device revoked.";
			if (this.outcome.sessionEnded) this.notice += " This session ended. Use the local CLI to continue.";
		} else if (this.outcome?.state === "unknown") this.notice = this.freshAttempted ? "The original result is still unconfirmed. Check with tmt remote settings or tmt remote devices." : "This change could not be confirmed. Check its original result before making another change.";
		else if (this.outcome?.state === "refused") this.notice = this.outcome.reason === "REMOTE_MANAGEMENT_CAPACITY" ? "Browser change limit reached. Use the local CLI; do not retry or reset storage." : this.outcome.reason === "REMOTE_MANAGEMENT_READ_ONLY" ? "This browser is read-only. Use the local CLI to make changes." : "Change refused. Check the current values or use the local CLI.";
	}
};
function accessRefused(error) {
	if (error instanceof ReopenSessionError) return error.reason === "revoked" || error.reason === "expired";
	return error instanceof RefusalError && ["REMOTE_DEVICE_REVOKED", "REMOTE_GRANT_EXPIRED"].includes(error.code);
}
//#endregion
//#region src/settings-page.ts
function element(id) {
	const found = document.getElementById(id);
	if (!found) throw new Error(`Settings page lacks #${id}.`);
	return found;
}
var opening = element("opening");
var mode = element("limit-mode");
var custom = element("limit-custom");
var devices = element("devices");
var refresh = element("refresh");
var more = element("more");
var first = element("first");
var rows = /* @__PURE__ */ new Map();
var initialized = false;
var choices;
var confirmation;
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
	const signature = targetSignature(target);
	if (confirmation?.button === button && confirmation.signature === signature) {
		cancelConfirmation();
		return true;
	}
	cancelConfirmation();
	const panel = rows.get(target.clientId).querySelector("[data-confirmation]");
	const consequence = panel.querySelector("p");
	consequence.textContent = kind === "talk" ? `${target.name} will be able to send to its permitted agents. Press Enable sending again to allow it. You can disable sending later.` : `Revoking ${target.name} ends ${target.thisBrowser ? "this browser’s" : "its"} access. Press Revoke again to end it. Pair again to restore access.`;
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
var page;
var registryState = remoteRegistryPage(() => {
	if (!page) return "";
	if (page.busy) return "A request is in progress. Wait for its outcome.";
	if (page.outcome?.state === "unknown") return "Another change is still unconfirmed. Check its original result first.";
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
	return refusal?.code === "REMOTE_SESSION_LIMIT" ? `This device already has ${refusal.limit} open sessions, the limit. Close another Remote tab, or change the limit with tmt remote settings sessions-per-device <n> (or off).` : void 0;
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
function saveState(form, hint, changed, valid) {
	const button = element(form).querySelector("button[type=submit]");
	button.disabled = !page.writable || !changed || !valid;
	element(hint).hidden = !page.writable || changed;
	const described = button.getAttribute("aria-describedby");
	if (!element(hint).hidden) button.setAttribute("aria-describedby", [described, hint].filter(Boolean).join(" "));
}
function recoverOriginal() {
	run(async () => {
		await page.recover();
		if (page.access === "live") await refreshView();
	});
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
	label.textContent = "Check original result";
	recover.append(label);
	recover.addEventListener("click", recoverOriginal);
	feedback.append(status, original, recover);
	return feedback;
}
function renderOutcome() {
	const origin = page.intent?.kind === "setting" ? page.intent.input.setting : page.intent?.input.clientId;
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
		original.textContent = selected && page.intent && (page.outcome?.state === "unknown" || page.outcome?.state === "refused") ? `Original operation ${page.intent.input.operationId}` : "";
		original.hidden = !original.textContent;
		recover.hidden = !selected || !page.canRecover;
		recover.disabled = page.busy;
		recover.setAttribute("aria-busy", String(page.busy));
	}
}
function render() {
	registryState(page.access === "live" ? "Connected" : page.access === "lost" ? "Access ended" : page.access === "checking" ? "Checking" : "Unconfirmed");
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
	element("read-only").hidden = editable;
	element("access-notice").dataset.tone = page.access === "live" ? editable ? "working" : "review" : page.access === "checking" ? "waiting" : "blocked";
	const reason = capacity ?? (page.busy ? "A request is in progress. Wait for its outcome." : !editable ? "Changes are unavailable in this browser. Use the local CLI." : page.outcome?.state === "unknown" ? "Another change is still unconfirmed. Check its original result first." : page.outcome?.state === "refused" && page.outcome.reason === "REMOTE_MANAGEMENT_CAPACITY" ? "Browser change limit reached. Use the local CLI; do not retry or reset storage." : "");
	element("controls-reason").textContent = reason;
	element("controls-reason").hidden = !reason;
	for (const button of document.querySelectorAll("button")) {
		if (button.closest("#remotes, #remote-switcher")) continue;
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
	for (const button of document.querySelectorAll("form button:not(.settings-choice):not([role=\"option\"])")) button.disabled = !page.writable;
	if (page.settings) {
		choices ??= [settingsChoice(opening, element("opening-options")), settingsChoice(mode, element("limit-mode-options"))];
		for (const update of choices) update();
	}
	saveState("opening-form", "opening-unchanged", openingChanged(), opening.checkValidity());
	saveState("limit-form", "limit-unchanged", limitChanged(), mode.value !== "custom" || custom.checkValidity());
	refresh.disabled = page.busy;
	more.hidden = !page.devices?.nextCursor;
	more.disabled = page.busy;
	first.hidden = page.onFirstPage;
	first.disabled = page.busy;
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
				revokeLabel.textContent = "Revoke";
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
					});
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
					});
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
					});
				});
				form.append(field, save, talk, revoke);
				const panel = document.createElement("div");
				panel.className = "device-confirmation";
				panel.dataset.confirmation = "";
				panel.id = `confirm-${device.clientId}`;
				panel.hidden = true;
				const consequence = document.createElement("p");
				consequence.id = `${panel.id}-consequence`;
				consequence.setAttribute("role", "status");
				const cancel = document.createElement("button");
				cancel.type = "button";
				cancel.className = "tmt-ui-action";
				const cancelLabel = document.createElement("span");
				cancelLabel.className = "tmt-ui-action-label";
				cancelLabel.textContent = "Cancel";
				cancel.append(cancelLabel);
				cancel.addEventListener("click", () => cancelConfirmation(true));
				panel.append(consequence, cancel);
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
			row.querySelector(".device-summary").textContent = `${device.name}${device.thisBrowser ? " · This device" : ""} · ${device.kind} · ${device.revoked ? "Revoked" : `Paired · Sending ${device.talkEnabled ? "on" : "off"}`} · ${device.liveSessionCount} live ${device.liveSessionCount === 1 ? "session" : "sessions"} · Last activity ${device.lastActivityAtMs === null ? "unavailable" : new Date(device.lastActivityAtMs).toLocaleString()}`;
			row.querySelector("[data-action=\"talk\"] .tmt-ui-action-label").textContent = device.talkEnabled ? "Disable sending" : "Enable sending";
			row.querySelector("form").setAttribute("aria-label", `Device ${device.name}`);
			const name = row.querySelector("input");
			name.removeAttribute("aria-labelledby");
			name.setAttribute("aria-label", `Device name for ${device.name}`);
			row.querySelector("button[type=submit]").setAttribute("aria-label", device.thisBrowser ? `Rename this device (${device.name})` : `Rename ${device.name}`);
			row.querySelector("[data-action=\"talk\"]").setAttribute("aria-label", `${device.talkEnabled ? "Disable" : "Enable"} sending for ${device.name}`);
			row.querySelector("button[data-action=\"revoke\"]").setAttribute("aria-label", `Revoke ${device.name}`);
			name.disabled = !editable || device.revoked;
			if (editable) name.removeAttribute("aria-describedby");
			else name.setAttribute("aria-describedby", "read-only");
			const disabledReason = element(`device-reason-${device.clientId}`);
			disabledReason.textContent = page.outcome?.state === "unknown" && page.intent?.kind !== "setting" && page.intent?.input.clientId === device.clientId ? "" : device.revoked ? "This device is revoked." : reason;
			disabledReason.hidden = !disabledReason.textContent;
			for (const button of row.querySelectorAll("form button")) {
				button.disabled = !page.writable || device.revoked;
				if (disabledReason.textContent) button.setAttribute("aria-describedby", disabledReason.id);
				else if (confirmation?.button === button) button.setAttribute("aria-describedby", `confirm-${device.clientId}-consequence`);
				else button.removeAttribute("aria-describedby");
				button.setAttribute("aria-busy", String(page.busy));
			}
		}
	}
	renderOutcome();
}
async function run(action) {
	cancelConfirmation();
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
async function change(intent) {
	await run(() => page.submit(intent));
	if (page.outcome?.state === "committed" && !page.outcome.sessionEnded) await run(() => refreshView());
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
	});
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
	});
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
function navigate(cursor) {
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
	run(() => refreshView(cursor));
}
more.addEventListener("click", () => navigate(page.devices?.nextCursor ?? null));
first.addEventListener("click", () => navigate(null));
for (const button of document.querySelectorAll("[data-recover]")) button.addEventListener("click", recoverOriginal);
try {
	let session = await reopenSession();
	page = new ManagementPage(management(session), async () => {
		session = await reopenSession(session, { retry: "bounded" });
		return management(session);
	});
	await run(() => refreshView());
} catch (error) {
	const capacity = capacityCopy(error instanceof RefusalError ? error : void 0);
	const ended = error instanceof RefusalError && [
		"REMOTE_DEVICE_REVOKED",
		"REMOTE_GRANT_EXPIRED",
		"REMOTE_SESSION_ENDED",
		"REMOTE_SESSION_EVICTED"
	].includes(error.code);
	commandNotice(element("access"), capacity ?? (ended ? "Current browser access refused. Use the local CLI." : "Current browser access unconfirmed. Pair locally with tmt remote pair, or use the local CLI."));
	element("access-announcement").textContent = capacity ?? (ended ? "Access ended" : "Access unconfirmed");
	registryState(error instanceof RefusalError && ["REMOTE_DEVICE_REVOKED", "REMOTE_GRANT_EXPIRED"].includes(error.code) ? "Access ended" : "Unconfirmed");
	refresh.disabled = true;
	element("access-notice").dataset.tone = "blocked";
	element("controls-reason").textContent = capacity ?? "Current access is unavailable. Use the local CLI.";
}
//#endregion
