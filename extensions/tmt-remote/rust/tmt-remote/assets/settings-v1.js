import { ClientError, RefusalError, budget, management, reopenSession } from "/sdk/remote-v1.js";
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
function renderFirestore(target, access, view) {
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
				row.append(node("span", `${firestoreItems[item.item]}: `), mark(item.state));
				if (item.reason) row.append(node("p", firestoreReasons[item.item][item.reason][0]));
				if (item.next) {
					const hint = node("p", "Next command on the machine that runs Remote: ");
					hint.append(command(item.next));
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
	busy = false;
	notice = "";
	freshAttempted = false;
	cursor = null;
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
		this.busy = true;
		this.firestore = void 0;
		this.firestoreAccess = "checking";
		try {
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
			try {
				this.firestore = await this.client.settings({ firestore: true });
				this.firestoreAccess = "confirmed";
			} catch {
				this.firestoreAccess = "unconfirmed";
			}
		} catch (error) {
			this.firestoreAccess = "unconfirmed";
			this.access = accessRefused(error) ? "lost" : "unconfirmed";
			if (this.outcome) this.describeOutcome();
			else this.notice = "Current access could not be confirmed. Use the local CLI.";
		} finally {
			this.busy = false;
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
		this.notice = "Saving…";
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
			this.notice = "Outcome unknown. Read the original operation; do not submit it again.";
		} finally {
			this.busy = false;
		}
	}
	/** One explicit fresh live-grant admission attempt, then only the original read.
	* Access refusal never establishes the prior mutation's outcome.
	*/
	async recover() {
		if (!this.intent || this.busy || this.freshAttempted) return;
		this.freshAttempted = true;
		this.busy = true;
		try {
			this.client = await this.reopen();
			this.access = "live";
			try {
				this.outcome = await this.client.operation(this.intent.input.operationId);
				this.describeOutcome();
			} catch {
				this.notice = "Original outcome unavailable or unconfirmed. Confirm with tmt remote devices or tmt remote settings.";
			}
		} catch (error) {
			this.access = accessRefused(error) ? "lost" : "unconfirmed";
			this.notice = accessRefused(error) ? "Current access lost. Prior outcome remains unknown unless already acknowledged. Confirm with tmt remote devices." : "Current access unconfirmed. Prior outcome remains unknown unless already acknowledged. Confirm with the local CLI.";
		} finally {
			this.busy = false;
		}
	}
	get canRecover() {
		return !!this.intent && !this.busy && !this.freshAttempted && (this.outcome?.state === "unknown" || this.access !== "live");
	}
	describeOutcome() {
		if (this.outcome?.state === "committed") this.notice = this.intent?.kind === "setting" ? "Saved. Session limits apply at the next session open." : "Device change committed.";
		else if (this.outcome?.state === "unknown") this.notice = "Outcome unknown. Read the original operation; do not submit it again.";
		else if (this.outcome?.state === "refused") this.notice = this.outcome.reason === "REMOTE_MANAGEMENT_CAPACITY" ? "Browser management operation limit reached. Use tmt remote settings or tmt remote devices; do not retry or reset storage." : this.outcome.reason === "REMOTE_MANAGEMENT_READ_ONLY" ? "This browser is read-only. Use the local CLI." : "Change refused. Check the admitted values or use the local CLI.";
	}
};
function accessRefused(error) {
	return error instanceof RefusalError && [
		"REMOTE_CLOSED",
		"REMOTE_SESSION_ENDED",
		"REMOTE_SESSION_EVICTED"
	].includes(error.code);
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
var recover = element("recover");
var refresh = element("refresh");
var more = element("more");
var first = element("first");
var rows = /* @__PURE__ */ new Map();
var initialized = false;
var page;
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
function render() {
	renderFirestore(element("firestore-content"), page.firestoreAccess, page.firestore);
	element("access").textContent = {
		checking: "Checking current access…",
		live: "Current browser access confirmed.",
		lost: "Current browser access refused.",
		unconfirmed: "Current browser access unconfirmed."
	}[page.access];
	const editable = page.access === "live" && page.settings?.capabilities.settingsWrite === true;
	element("read-only").hidden = editable;
	element("access-notice").dataset.tone = page.access === "live" ? editable ? "working" : "review" : page.access === "checking" ? "waiting" : "blocked";
	element("outcome-notice").dataset.tone = page.busy ? "waiting" : page.outcome?.state === "committed" ? "working" : page.outcome?.state === "unknown" || page.outcome?.state === "refused" ? "blocked" : "review";
	const reason = page.busy ? "A request is in progress. Wait for its outcome." : !editable ? "Changes are unavailable in this browser. Use the local CLI." : page.outcome?.state === "unknown" ? "The original outcome is unknown. Read it before another change." : page.outcome?.state === "refused" && page.outcome.reason === "REMOTE_MANAGEMENT_CAPACITY" ? "Browser management operation limit reached. Use the local CLI; do not retry or reset storage." : "";
	element("controls-reason").textContent = reason;
	element("controls-reason").hidden = !reason;
	for (const button of document.querySelectorAll("button")) {
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
	for (const button of document.querySelectorAll("form button")) button.disabled = !page.writable;
	refresh.disabled = page.busy;
	recover.hidden = !page.canRecover;
	more.hidden = !page.devices?.nextCursor;
	more.disabled = page.busy;
	first.hidden = page.onFirstPage;
	first.disabled = page.busy;
	commandNotice(element("outcome"), `${page.outcome?.state && page.outcome.state !== "unknown" ? `${page.outcome.state}: ` : ""}${page.notice || "No change submitted."}`);
	element("original").textContent = page.intent ? `Original operation ${page.intent.input.operationId}` : "";
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
				revoke.className = "tmt-ui-action";
				revoke.dataset.variant = "destructive";
				const revokeLabel = document.createElement("span");
				revokeLabel.className = "tmt-ui-action-label";
				revokeLabel.textContent = "Revoke";
				revoke.append(revokeLabel);
				revoke.addEventListener("click", () => {
					if (!page.writable) return;
					const target = page.devices?.devices.find((item) => item.clientId === device.clientId);
					if (!target || target.revoked) return;
					if (confirm(`Revoke ${target.name}${target.thisBrowser ? " (this device)" : ""}?`)) change({
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
				talk.addEventListener("click", () => {
					if (!page.writable) return;
					const target = page.devices?.devices.find((item) => item.clientId === device.clientId);
					if (!target || target.revoked) return;
					if (!target.talkEnabled && !confirm(`Allow ${target.name} to send to its permitted agents?`)) return;
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
				const disabledReason = document.createElement("p");
				disabledReason.id = `device-reason-${device.clientId}`;
				disabledReason.className = "tmt-ui-field-description";
				row.append(summary, form, disabledReason);
				rows.set(device.clientId, row);
				devices.append(row);
			}
			row.querySelector(".device-summary").textContent = `${device.name}${device.thisBrowser ? " · This device" : ""} · ${device.kind} · ${device.revoked ? "Revoked" : `Paired · Sending ${device.talkEnabled ? "on" : "off"}`} · ${device.liveSessionCount} live ${device.liveSessionCount === 1 ? "session" : "sessions"} · Last activity ${device.lastActivityAtMs === null ? "unavailable" : new Date(device.lastActivityAtMs).toLocaleString()}`;
			row.querySelector("[data-action=\"talk\"] .tmt-ui-action-label").textContent = device.talkEnabled ? "Disable sending" : "Enable sending";
			const name = row.querySelector("input");
			name.disabled = !editable || device.revoked;
			if (editable) name.removeAttribute("aria-describedby");
			else name.setAttribute("aria-describedby", "read-only");
			const disabledReason = element(`device-reason-${device.clientId}`);
			disabledReason.textContent = device.revoked ? "This device is revoked." : reason;
			disabledReason.hidden = !disabledReason.textContent;
			for (const button of row.querySelectorAll("button")) {
				button.disabled = !page.writable || device.revoked;
				if (disabledReason.textContent) button.setAttribute("aria-describedby", disabledReason.id);
				else button.removeAttribute("aria-describedby");
				button.setAttribute("aria-busy", String(page.busy));
			}
		}
	}
}
async function run(action) {
	const pending = action();
	render();
	try {
		await pending;
	} catch (error) {
		page.notice = error instanceof Error ? error.message : "Action unavailable.";
	}
	render();
}
async function change(intent) {
	await run(() => page.submit(intent));
	if (page.outcome?.state === "committed" && !page.outcome.sessionEnded) await run(() => page.refresh());
}
element("opening-form").addEventListener("submit", (event) => {
	event.preventDefault();
	if (!page.writable) return;
	if (opening.value === "on" === page.settings?.settings.open) return;
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
	if (mode.value === "default") return;
	const value = mode.value === "off" ? null : custom.value;
	if (value === page.settings?.settings.sessionsPerDevice && page.settings.settings.sessionsPerDeviceSource === "settings.json") return;
	change({
		kind: "setting",
		input: {
			operationId: crypto.randomUUID(),
			setting: "sessions-per-device",
			value
		}
	});
});
mode.addEventListener("change", render);
refresh.addEventListener("click", () => void run(() => page.refresh()));
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
	run(() => page.refresh(cursor));
}
more.addEventListener("click", () => navigate(page.devices?.nextCursor ?? null));
first.addEventListener("click", () => navigate(null));
recover.addEventListener("click", () => void run(async () => {
	await page.recover();
	if (page.access === "live") await page.refresh();
}));
try {
	let session = await reopenSession();
	page = new ManagementPage(management(session), async () => {
		session = await reopenSession(session);
		return management(session);
	});
	await run(() => page.refresh());
} catch (error) {
	commandNotice(element("access"), error instanceof RefusalError ? "Current browser access refused. Use the local CLI." : "Current browser access unconfirmed. Pair locally with tmt remote pair, or use the local CLI.");
	refresh.disabled = true;
	element("access-notice").dataset.tone = "blocked";
	element("controls-reason").textContent = "Current access is unavailable. Use the local CLI.";
}
//#endregion
