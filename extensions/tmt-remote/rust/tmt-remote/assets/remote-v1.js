//#region \0rolldown/runtime.js
var __defProp = Object.defineProperty;
var __exportAll = (all, no_symbols) => {
	let target = {};
	for (var name in all) __defProp(target, name, {
		get: all[name],
		enumerable: true
	});
	if (!no_symbols) __defProp(target, Symbol.toStringTag, { value: "Module" });
	return target;
};
//#endregion
//#region src/canonical-bytes.ts
var encoder$2 = new TextEncoder();
var UUID$3 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
var ADDON_ORIGIN = /^chrome-extension:\/\/[a-p]{32}$/;
var DOOR_ORIGIN = /^http:\/\/127\.0\.0\.1:([1-9][0-9]{0,4})$/;
var BASE32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
function requireValue$1(condition, label) {
	if (!condition) throw new Error(`Invalid ${label}.`);
}
function text$1(value) {
	requireValue$1(typeof value === "string", "text");
	for (let index = 0; index < value.length; index++) {
		const unit = value.charCodeAt(index);
		if (unit >= 55296 && unit <= 56319) {
			const next = value.charCodeAt(++index);
			requireValue$1(next >= 56320 && next <= 57343, "UTF-8 surrogate");
		} else requireValue$1(unit < 56320 || unit > 57343, "UTF-8 surrogate");
	}
	return encoder$2.encode(value);
}
function count$1(value) {
	requireValue$1(Number.isInteger(value) && value >= 0 && value <= 4294967295, "u32 length");
	const result = /* @__PURE__ */ new Uint8Array(4);
	new DataView(result.buffer).setUint32(0, value, false);
	return result;
}
function concat(parts) {
	const length = parts.reduce((sum, part) => sum + part.length, 0);
	requireValue$1(Number.isSafeInteger(length), "combined length");
	const result = new Uint8Array(length);
	let offset = 0;
	for (const part of parts) {
		result.set(part, offset);
		offset += part.length;
	}
	return result;
}
function lp(value) {
	return concat([count$1(value.length), value]);
}
var lpText = (value) => lp(text$1(value));
function uuid(value) {
	requireValue$1(value.length === 36 && UUID$3.test(value), "UUIDv4");
}
function binary(value, length) {
	requireValue$1(value instanceof Uint8Array && value.length === length, `binary length ${length}`);
	return lp(value);
}
var addonOrigin = (value) => value.length === 51 && ADDON_ORIGIN.test(value);
function doorOrigin(value) {
	const port = DOOR_ORIGIN.exec(value)?.[1];
	return port !== void 0 && Number(port) <= 65535;
}
function origin(value) {
	requireValue$1(text$1(value).length <= 128 && (value === "cli" || addonOrigin(value) || doorOrigin(value)), "origin");
}
/** Hash the exact supplied payload bytes, without parsing or reserializing them. */
async function envelopeSigningBytes(value) {
	requireValue$1(value.version === 1 && value.profile === "local-v1", "envelope profile/version");
	requireValue$1([
		"request",
		"response",
		"control"
	].includes(value.kind), "envelope kind");
	for (const id of [
		value.id,
		value.machineId,
		value.windowId,
		value.clientId
	]) uuid(id);
	if (value.kind === "response") uuid(value.correlationId ?? "");
	else requireValue$1(value.correlationId === null, "correlationId");
	requireValue$1(typeof value.sequence === "string" && /^(0|[1-9][0-9]{0,19})$/.test(value.sequence), "sequence spelling");
	requireValue$1(BigInt(value.sequence).toString() === value.sequence && BigInt(value.sequence) <= 18446744073709551615n, "sequence bound");
	if (value.kind === "control") requireValue$1([
		"session.open",
		"subscribe",
		"ack"
	].includes(value.operation), "control operation");
	if (value.kind === "control" && value.operation === "session.open") requireValue$1(value.sessionId === "new" && value.sequence === "0", "session.open");
	else if (value.kind === "response" && value.operation === "session.open" && value.sessionId === "new") requireValue$1(value.sequence === "1", "rejected session.open");
	else {
		uuid(value.sessionId);
		requireValue$1(value.sequence !== "0", "normal sequence");
	}
	requireValue$1(Number.isSafeInteger(value.timestampMs) && value.timestampMs >= 0, "timestampMs");
	origin(value.origin);
	requireValue$1(text$1(value.operation).length > 0, "operation");
	requireValue$1(value.payload instanceof Uint8Array, "payload bytes");
	const payload = new Uint8Array(value.payload);
	const fields = [
		"tmt-message-v1",
		String(value.version),
		value.profile,
		value.kind,
		value.id,
		value.correlationId ?? "",
		value.machineId,
		value.windowId,
		value.clientId,
		value.sessionId,
		value.sequence,
		String(value.timestampMs),
		value.origin,
		value.operation
	].map(lpText);
	const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", payload));
	return concat([...fields, lp(digest)]);
}
/** Encode a decoded enrollment candidate; no key validity, proof or authority is established. */
function enrollmentSigningBytes(value) {
	requireValue$1(value.profile === "local-v1", "enrollment profile");
	for (const id of [
		value.machineId,
		value.windowId,
		value.offerId
	]) uuid(id);
	origin(value.origin);
	requireValue$1(value.kind === "addon" && addonOrigin(value.origin) || value.kind === "browser" && doorOrigin(value.origin) || value.kind === "cli" && value.origin === "cli", "kind/origin");
	const name = text$1(value.name);
	requireValue$1(name.length >= 1 && name.length <= 64 && value.name.trim().length > 0 && !Array.from(value.name).some((character) => {
		const point = character.codePointAt(0);
		return point <= 31 || point >= 127 && point <= 159;
	}), "name");
	return concat([
		lpText("tmt-device-pair-v1"),
		lpText(value.profile),
		lpText(value.machineId),
		lpText(value.windowId),
		lpText(value.offerId),
		binary(value.serverChallenge, 16),
		binary(value.clientNonce, 16),
		lpText(value.kind),
		lpText(value.origin),
		lp(name),
		binary(value.publicKey, 32)
	]);
}
/** Frame already canonical enrollment bytes and an already computed full MAC; do not compute it. */
function enrollmentPossessionSigningBytes(enrollment, mac) {
	requireValue$1(enrollment instanceof Uint8Array, "enrollment bytes");
	return concat([
		lpText("tmt-device-pair-possession-v1"),
		lp(enrollment),
		binary(mac, 32)
	]);
}
/** Distinct from enrollment: a live device cancels an automatic offer, never enrolls. */
function enrollmentCancellationSigningBytes(enrollment, mac) {
	requireValue$1(enrollment instanceof Uint8Array, "enrollment bytes");
	return concat([
		lpText("tmt-device-pair-cancel-v1"),
		lp(enrollment),
		binary(mac, 32)
	]);
}
var EXTENSION = /^[a-z][a-z0-9-]{0,31}$/;
/** `tmt-ext-cert-v1` signing input: a device key certifies an extension key. */
function extCertSigningBytes(value) {
	requireValue$1(typeof value.extension === "string" && EXTENSION.test(value.extension), "extension");
	requireValue$1(value.purpose === "sign" || value.purpose === "enc", "purpose");
	requireValue$1(Number.isSafeInteger(value.issuedAtMs) && value.issuedAtMs >= 0, "certificate time");
	return concat([
		lpText("tmt-ext-cert-v1"),
		lpText(value.extension),
		lpText(value.purpose),
		binary(value.publicKey, 32),
		lpText(String(value.issuedAtMs))
	]);
}
/** HMAC input for `K_response`, keyed by the pairing code. */
function responseKeyInput(enrollment) {
	requireValue$1(enrollment instanceof Uint8Array, "enrollment bytes");
	return concat([lpText("tmt-device-pair-response-key-v1"), lp(enrollment)]);
}
/** HMAC input for `serverProof`, keyed by `K_response`, over the exact receipt bytes. */
function serverProofInput(receipt) {
	requireValue$1(receipt instanceof Uint8Array, "receipt bytes");
	return concat([lpText("tmt-device-pair-response-v1"), lp(receipt)]);
}
/**
* Decode a pairing code after removing ASCII spaces and hyphens only. Anything
* else, a wrong length or nonzero unused bits refuses.
*/
function pairingCode(textValue) {
	const symbols = textValue.replace(/[ -]/g, "");
	requireValue$1(symbols.length === 26, "pairing code length");
	const code = /* @__PURE__ */ new Uint8Array(16);
	let buffer = 0;
	let bits = 0;
	let index = 0;
	for (const symbol of symbols) {
		const value = BASE32.indexOf(symbol);
		requireValue$1(value >= 0, "pairing code alphabet");
		buffer = (buffer << 5 | value) & 4095;
		bits += 5;
		if (bits >= 8) {
			bits -= 8;
			code[index++] = buffer >> bits & 255;
		}
	}
	requireValue$1(index === 16 && (buffer & (1 << bits) - 1) === 0, "pairing code unused bits");
	return code;
}
/**
* Four 11-bit indexes into the pinned BIP-39 English list (bitcoin/bips
* ce1862ac, SHA-256 2f5eed53…), from the first 44 bits of the key fingerprint.
* Comparison text only, never a recovery mnemonic.
*/
async function fingerprintIndexes(publicKey) {
	const input = new Uint8Array(concat([lpText("tmt-local-key-fingerprint-v1"), binary(publicKey, 32)]));
	const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", input));
	let bits = 0n;
	for (const byte of digest.subarray(0, 6)) bits = bits << 8n | BigInt(byte);
	bits >>= 4n;
	const index = (shift) => Number(bits >> shift & 2047n);
	return [
		index(33n),
		index(22n),
		index(11n),
		index(0n)
	];
}
var BASE64URL = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
/** Unpadded RFC 4648 base64url. */
function base64url(bytes) {
	requireValue$1(bytes instanceof Uint8Array, "base64url bytes");
	let text = "";
	for (let index = 0; index < bytes.length; index += 3) {
		const chunk = bytes[index] << 16 | (bytes[index + 1] ?? 0) << 8 | (bytes[index + 2] ?? 0);
		const symbols = Math.min(4, Math.ceil((bytes.length - index) * 8 / 6));
		for (let symbol = 0; symbol < symbols; symbol++) text += BASE64URL[chunk >> 18 - 6 * symbol & 63];
	}
	return text;
}
/**
* Decode exactly `length` bytes of strict base64url: no padding, no other
* alphabet or whitespace, and zero unused bits, so each value has one spelling.
*/
function base64urlBytes(textValue, length) {
	requireValue$1(typeof textValue === "string" && textValue.length % 4 !== 1, "base64url length");
	const bytes = new Uint8Array(Math.floor(textValue.length * 6 / 8));
	let buffer = 0;
	let bits = 0;
	let index = 0;
	for (const symbol of textValue) {
		const value = BASE64URL.indexOf(symbol);
		requireValue$1(value >= 0, "base64url alphabet");
		buffer = (buffer << 6 | value) & 16383;
		bits += 6;
		if (bits >= 8) {
			bits -= 8;
			bytes[index++] = buffer >> bits & 255;
		}
	}
	requireValue$1((buffer & (1 << bits) - 1) === 0, "base64url unused bits");
	requireValue$1(bytes.length === length, "base64url decoded length");
	return bytes;
}
//#endregion
//#region src/session-channel.ts
var owned$1 = (bytes) => new Uint8Array(bytes);
async function verifyEd25519(publicKey, message, signature) {
	const key = await crypto.subtle.importKey("raw", owned$1(publicKey), "Ed25519", false, ["verify"]);
	return signature.length === 64 && crypto.subtle.verify("Ed25519", key, owned$1(signature), owned$1(message));
}
var channels = /* @__PURE__ */ new WeakMap();
function registerChannel(session, paired, key, windowId, fetch) {
	channels.set(session, {
		paired: {
			...paired,
			machinePublicKey: new Uint8Array(paired.machinePublicKey)
		},
		key,
		windowId,
		sessionId: session.sessionId,
		fetch,
		clientSequence: 1n,
		machineSequence: 1n,
		tail: Promise.resolve(),
		ended: false
	});
}
var channelFor = (session) => channels.get(session);
/** Authenticate exact bytes and correlation before the operation parses its payload. */
async function verifyResponse(value, paired, expected) {
	function valid(condition) {
		if (!condition) throw new Error("Invalid machine response.");
	}
	valid(typeof value === "object" && value !== null && !Array.isArray(value));
	const reply = value;
	const text = (name) => {
		const value = reply[name];
		valid(typeof value === "string");
		return value;
	};
	valid(reply.version === 1 && text("profile") === "local-v1" && text("kind") === "response" && text("correlationId") === expected.id && text("machineId") === paired.machineId && text("windowId") === expected.windowId && text("clientId") === paired.clientId && text("origin") === paired.origin && text("operation") === expected.operation && Number.isSafeInteger(reply.timestampMs));
	const sessionId = text("sessionId");
	valid(expected.sessionId === void 0 || sessionId === expected.sessionId);
	const payloadText = text("payload");
	const payload = base64urlBytes(payloadText, Math.floor(payloadText.length * 6 / 8));
	const signed = await envelopeSigningBytes({
		version: 1,
		profile: "local-v1",
		kind: "response",
		id: text("id"),
		correlationId: expected.id,
		machineId: paired.machineId,
		windowId: expected.windowId,
		clientId: paired.clientId,
		sessionId,
		sequence: text("sequence"),
		timestampMs: reply.timestampMs,
		origin: paired.origin,
		operation: expected.operation,
		payload
	});
	const sequence = BigInt(text("sequence"));
	valid(sequence > expected.after);
	valid(await verifyEd25519(paired.machinePublicKey, signed, base64urlBytes(text("signature"), 64)));
	return {
		payload,
		sequence,
		sessionId
	};
}
//#endregion
//#region src/operations.ts
/** Unknown outcome only; the caller retains its operation ID for read-only recovery. */
var ClientError = class extends Error {
	code;
	operationId;
	constructor(code, message, operationId) {
		super(message);
		this.code = code;
		this.operationId = operationId;
		this.name = "ClientError";
	}
};
/** Known refusal; send/operation return pre-effect codes as state instead. */
var RefusalError = class extends Error {
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
var SequenceMismatch = class extends Error {};
var PRE_EFFECT = /* @__PURE__ */ new Set([
	"REMOTE_SCOPE_DENIED",
	"REMOTE_INPUT_INVALID",
	"REMOTE_RATE_LIMITED",
	"REMOTE_INTENT_CONFLICT",
	"REMOTE_CLOSED",
	"REMOTE_SESSION_ENDED",
	"REMOTE_SESSION_EVICTED"
]);
var MANAGEMENT_PRE_EFFECT = /* @__PURE__ */ new Set([
	"REMOTE_SCOPE_DENIED",
	"REMOTE_INPUT_INVALID",
	"REMOTE_RATE_LIMITED",
	"REMOTE_INTENT_CONFLICT",
	"REMOTE_INPUT_TOO_LARGE",
	"REMOTE_MANAGEMENT_READ_ONLY",
	"REMOTE_DEVICE_REVOKED",
	"REMOTE_DEVICE_NOT_FOUND",
	"REMOTE_SETTINGS_UNAVAILABLE",
	"REMOTE_MANAGEMENT_CAPACITY",
	"REMOTE_DEVICE_LIST_CHANGED",
	"REMOTE_DEVICE_REMOVAL_LIMIT"
]);
var UUID$2 = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
var V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
var coreId = (value) => typeof value === "string" && UUID$2.test(value) && value !== "00000000-0000-0000-0000-000000000000";
var requestId = (value) => typeof value === "string" && value.startsWith("req_") && coreId(value.slice(4));
var utf8$1 = new TextEncoder();
var strictUtf8$1 = new TextDecoder("utf-8", { fatal: true });
var MAX_SEQUENCE = 18446744073709551615n;
function valid$6(condition) {
	if (!condition) throw new Error("Invalid operation response.");
}
function input$1(condition) {
	if (!condition) throw new TypeError("Invalid Remote operation input.");
}
function record(value) {
	valid$6(typeof value === "object" && value !== null && !Array.isArray(value));
	return value;
}
function reason(value) {
	if (value.reason === void 0) return {};
	valid$6(typeof value.reason === "string" && utf8$1.encode(value.reason).length <= 256);
	return { reason: value.reason };
}
var USER_NAME_REASONS = [
	"USER_NAME_UNAVAILABLE",
	"USER_NAME_INVALID",
	"USER_NAME_TAKEN"
];
function text(value) {
	return typeof value === "string" && value.length > 0 && utf8$1.encode(value).length <= 512;
}
/** Optional, and exact when present: a notice that does not parse makes the whole reply unverifiable. */
function identity$1(value) {
	if (value.identity === void 0) return {};
	const notice = record(value.identity);
	const keys = Object.keys(notice).sort().join();
	if (notice.state === "anonymous") {
		valid$6(keys === "message,reason,state" && USER_NAME_REASONS.includes(notice.reason));
		valid$6(text(notice.message));
		return { identity: {
			state: "anonymous",
			reason: notice.reason,
			message: notice.message
		} };
	}
	valid$6(notice.state === "person" && keys === "name,renamedFrom,state");
	valid$6(text(notice.name) && text(notice.renamedFrom));
	return { identity: {
		state: "person",
		name: notice.name,
		renamedFrom: notice.renamedFrom
	} };
}
function sendState(value, id) {
	return {
		...sendStateBase(value, id),
		...identity$1(record(value))
	};
}
function sendStateBase(value, id) {
	const state = record(value);
	valid$6(state.operationId === id);
	switch (state.state) {
		case "held": return {
			state: "held",
			operationId: id
		};
		case "accepted":
			valid$6(requestId(state.requestId));
			return {
				state: "accepted",
				operationId: id,
				requestId: state.requestId
			};
		case "uncertain":
			valid$6(state.requestId === void 0 || requestId(state.requestId));
			return {
				state: "uncertain",
				operationId: id,
				...state.requestId === void 0 ? {} : { requestId: state.requestId }
			};
		case "refused":
		case "cancelled": return {
			state: state.state,
			operationId: id,
			...reason(state)
		};
		default: throw new Error("Invalid send state.");
	}
}
function resultState(value, id) {
	const state = record(value);
	valid$6(state.requestId === id);
	switch (state.state) {
		case "pending": return {
			state: "pending",
			requestId: id
		};
		case "replied":
			valid$6(typeof state.message === "string");
			return {
				state: "replied",
				requestId: id,
				message: state.message
			};
		case "unavailable": return {
			state: "unavailable",
			requestId: id,
			...reason(state)
		};
		default: throw new Error("Invalid result state.");
	}
}
function agents(value) {
	const rows = record(value).identities;
	valid$6(Array.isArray(rows));
	return rows.map((value) => {
		const row = record(value);
		valid$6(coreId(row.id) && typeof row.name === "string" && (row.presence === "active" || row.presence === "offline" || row.presence === "unknown"));
		const activity = row.activity;
		const observedAtMs = row.observedAtMs;
		return {
			id: row.id,
			name: row.name,
			presence: row.presence,
			...Object.hasOwn(row, "delivery") ? { delivery: row.delivery } : {},
			...row.runningDriver === "claude" || row.runningDriver === "codex" ? { runningDriver: row.runningDriver } : {},
			...activity !== null && typeof activity === "object" && !Array.isArray(activity) && "state" in activity && "sinceMs" in activity && (activity.state === "working" || activity.state === "idle" || activity.state === "unknown" || activity.state === "ended") && (activity.sinceMs === null || typeof activity.sinceMs === "number" && Number.isSafeInteger(activity.sinceMs) && activity.sinceMs >= 0) ? { activity: {
				state: activity.state,
				sinceMs: activity.sinceMs
			} } : {},
			...typeof observedAtMs === "number" && Number.isSafeInteger(observedAtMs) && observedAtMs >= 0 ? { observedAtMs } : {}
		};
	});
}
function remoteError(value, address) {
	if (!Object.hasOwn(value, "error")) return void 0;
	const error = record(value.error);
	valid$6(typeof error.code === "string" && /^[A-Z][A-Z0-9_]{0,63}$/.test(error.code) && typeof error.message === "string");
	const scope = error.scope;
	valid$6(scope === void 0 || error.code === "REMOTE_SCOPE_DENIED" && scope === "talk");
	const limit = error.limit;
	valid$6(limit === void 0 || typeof limit === "number" && Number.isSafeInteger(limit) && limit > 0);
	valid$6(!["REMOTE_SESSION_EVICTED", "REMOTE_SESSION_LIMIT"].includes(error.code) || limit !== void 0);
	let settingsUrl;
	if (error.settingsUrl !== void 0 && error.settingsUrl !== null) {
		valid$6(typeof error.settingsUrl === "string" && error.settingsUrl.length > 0 && error.settingsUrl.length <= 2048);
		const url = new URL(error.settingsUrl, address);
		const door = new URL(address);
		valid$6(url.origin === door.origin && !url.username && !url.password && ["http:", "https:"].includes(url.protocol));
		settingsUrl = url.href;
	}
	const retry = error.retryAfterMs;
	valid$6(retry === void 0 || typeof retry === "number" && Number.isSafeInteger(retry) && retry >= 0 && retry <= 6e4);
	if (error.code === "REMOTE_REPLAY") return new SequenceMismatch();
	valid$6(PRE_EFFECT.has(error.code) || [
		"REMOTE_GRANT_CHANGED",
		"REMOTE_SERVE_STOPPED",
		"REMOTE_SESSION_LIMIT",
		"REMOTE_INPUT_TOO_LARGE",
		"REMOTE_STATE_UNAVAILABLE",
		"REMOTE_CORE_UNAVAILABLE",
		"REMOTE_MANAGEMENT_READ_ONLY",
		"REMOTE_MANAGEMENT_UNAVAILABLE",
		"REMOTE_GRANT_EXPIRED",
		"REMOTE_DEVICE_REVOKED",
		"REMOTE_DEVICE_NOT_FOUND",
		"REMOTE_SETTINGS_UNAVAILABLE",
		"REMOTE_MANAGEMENT_CAPACITY",
		"REMOTE_DEVICE_LIST_CHANGED",
		"REMOTE_DEVICE_REMOVAL_LIMIT"
	].includes(error.code));
	return new RefusalError(error.code, retry, limit, settingsUrl, scope);
}
function enqueue(channel, action) {
	const pending = channel.tail.then(action);
	channel.tail = pending.then(() => void 0, () => void 0);
	return pending;
}
/** One attempt; an abandoned fetch can finish without touching live counters. */
async function attempt(channel, timeoutMs, operation, id, payload, sequence, parse, mutationOutcome = false) {
	const abort = new AbortController();
	let timer;
	let published = false;
	let failure = "transport_failure";
	try {
		return await Promise.race([(async () => {
			const envelope = {
				version: 1,
				profile: "local-v1",
				kind: "request",
				id,
				correlationId: null,
				machineId: channel.paired.machineId,
				windowId: channel.windowId,
				clientId: channel.paired.clientId,
				sessionId: channel.sessionId,
				sequence: sequence.toString(),
				timestampMs: Date.now(),
				origin: channel.paired.origin,
				operation
			};
			const signature = await channel.key.sign(await envelopeSigningBytes({
				...envelope,
				payload
			}));
			if (abort.signal.aborted) throw new Error("Abandoned signing.");
			published = true;
			channel.clientSequence = sequence + 1n;
			const send = channel.fetch;
			const response = await send(`${channel.paired.address}/append`, {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				credentials: "omit",
				signal: abort.signal,
				body: JSON.stringify({
					...envelope,
					payload: base64url(payload),
					signature: base64url(signature)
				})
			});
			if (abort.signal.aborted) throw new Error("Abandoned response.");
			if (response.status === 404) {
				channel.ended = true;
				if (mutationOutcome) throw new Error("Mutation acknowledgment is unconfirmed.");
				throw new RefusalError("REMOTE_SESSION_ENDED");
			}
			if (response.status !== 200) throw new Error("Unconfirmed transport response.");
			failure = "unverifiable_response";
			const reply = await verifyResponse(await response.json(), channel.paired, {
				id,
				windowId: channel.windowId,
				operation,
				sessionId: channel.sessionId,
				after: channel.machineSequence
			});
			if (abort.signal.aborted) throw new Error("Abandoned response.");
			channel.machineSequence = reply.sequence;
			const value = record(JSON.parse(strictUtf8$1.decode(reply.payload)));
			const error = remoteError(value, channel.paired.address);
			if (error) {
				if (error instanceof RefusalError && [
					"REMOTE_CLOSED",
					"REMOTE_GRANT_CHANGED",
					"REMOTE_SERVE_STOPPED",
					"REMOTE_SESSION_ENDED",
					"REMOTE_SESSION_EVICTED"
				].includes(error.code)) channel.ended = true;
				if (mutationOutcome && error instanceof RefusalError && !MANAGEMENT_PRE_EFFECT.has(error.code)) {
					failure = "outcome_unconfirmed";
					throw new Error("Signed failure does not establish the management outcome.");
				}
				throw error;
			}
			return parse(value);
		})(), new Promise((_, reject) => {
			timer = setTimeout(() => {
				failure = "timeout";
				abort.abort();
				reject(/* @__PURE__ */ new Error("Timed out."));
			}, timeoutMs);
		})]);
	} catch (error) {
		if (error instanceof RefusalError || error instanceof SequenceMismatch) throw error;
		if (!published) throw new TypeError("Remote operation could not be signed.");
		throw new ClientError(failure, {
			outcome_unconfirmed: "Remote management outcome is unconfirmed; observe the original operation.",
			timeout: "Remote outcome is unknown after timeout; observe the original operation.",
			transport_failure: "Remote transport outcome is unknown; observe the original operation.",
			unverifiable_response: "Remote response could not be verified; observe the original operation.",
			sequence_unavailable: "Remote sequence recovery failed; reopen before observing the original operation."
		}[failure]);
	} finally {
		clearTimeout(timer);
		abort.abort();
	}
}
function parseCapabilities(value) {
	const reply = record(value);
	valid$6(reply.version === 1 && reply.profile === "local-v1" && reply.binding === "loopback-http");
	valid$6(Array.isArray(reply.operations) && reply.operations.every((value) => typeof value === "string"));
	record(reply.limits);
}
/** Scope-free reads synchronize the lane; the caller's operation is never a probe. */
async function synchronize(channel, timeoutMs) {
	const unresolved = channel.uncertainSequence;
	if (unresolved === void 0) return;
	const unavailable = () => new ClientError("sequence_unavailable", "Remote sequence recovery failed; reopen before observing the original operation.");
	if (unresolved === "unavailable") throw unavailable();
	const id = crypto.randomUUID();
	const payload = utf8$1.encode("{}");
	try {
		try {
			await attempt(channel, timeoutMs, "capabilities", id, payload, channel.clientSequence, parseCapabilities);
		} catch (error) {
			if (!(error instanceof SequenceMismatch)) throw error;
			await attempt(channel, timeoutMs, "capabilities", id, payload, unresolved, parseCapabilities);
		}
		channel.uncertainSequence = void 0;
	} catch (error) {
		if (channel.ended) throw error;
		channel.uncertainSequence = "unavailable";
		throw unavailable();
	}
}
async function invoke(channel, timeoutMs, operation, id, payload, parse, mutationOutcome = false) {
	if (channel.ended || channel.clientSequence > MAX_SEQUENCE) throw new RefusalError("REMOTE_SESSION_ENDED");
	await synchronize(channel, timeoutMs);
	if (channel.clientSequence > MAX_SEQUENCE) throw new RefusalError("REMOTE_SESSION_ENDED");
	const sequence = channel.clientSequence;
	try {
		return await attempt(channel, timeoutMs, operation, id, payload, sequence, parse, mutationOutcome);
	} catch (error) {
		if (error instanceof SequenceMismatch) {
			channel.uncertainSequence = "unavailable";
			throw new ClientError("sequence_unavailable", "Remote sequence recovery failed; reopen before observing the original operation.");
		}
		if (!channel.ended && !(error instanceof TypeError)) channel.uncertainSequence = sequence;
		throw error;
	}
}
/** Package-internal primitive shared by agent and Remote management helpers.
* It retains the verified Session's serialized lane, signer and response correlation.
*/
function verifiedSessionRequest(session, options = {}) {
	const found = channelFor(session);
	if (!found) throw new TypeError("Use a verified openSession or reopenSession result.");
	const channel = found;
	const timeoutMs = options.timeoutMs ?? 4e4;
	input$1(Number.isSafeInteger(timeoutMs) && timeoutMs > 0 && timeoutMs <= 2147483647);
	return function call(operation, id, value, parse, mutationOutcome = false) {
		const payload = utf8$1.encode(JSON.stringify(value));
		return enqueue(channel, () => invoke(channel, timeoutMs, operation, id, payload, parse, mutationOutcome));
	};
}
/**
* Constructing this helper opens nothing. Reuse the owner's existing Session;
* after unknown send outcomes, observe operation(originalId) in that session.
* Recovery never dispatches, reopens, or automatically allocates a dispatch ID.
*/
function operations(session, options = {}) {
	const call = verifiedSessionRequest(session, options);
	return {
		listAgents: () => call("agents.list", crypto.randomUUID(), {}, agents),
		async send(value) {
			input$1(typeof value.operationId === "string" && V4.test(value.operationId) && coreId(value.agentId) && typeof value.message === "string" && strictUtf8$1.decode(utf8$1.encode(value.message)) === value.message);
			const { operationId, agentId, message } = value;
			try {
				return await call("dispatch.create", operationId, {
					version: 1,
					operation: "dispatch.create",
					originator: "anonymous",
					input: {
						operationId,
						recipientIds: [agentId],
						message,
						kind: "request"
					}
				}, (value) => sendState(value, operationId));
			} catch (error) {
				if (error instanceof RefusalError && PRE_EFFECT.has(error.code)) return {
					state: "refused",
					operationId,
					reason: error.code,
					...error.limit === void 0 ? {} : { limit: error.limit },
					...error.settingsUrl === void 0 ? {} : { settingsUrl: error.settingsUrl },
					...error.scope === void 0 ? {} : { scope: error.scope }
				};
				if (error instanceof ClientError) throw new ClientError(error.code, error.message, operationId);
				throw error;
			}
		},
		async operation(operationId) {
			input$1(coreId(operationId));
			try {
				return await call("operation.show", crypto.randomUUID(), { operationId }, (value) => sendState(value, operationId));
			} catch (error) {
				if (error instanceof RefusalError && PRE_EFFECT.has(error.code)) return {
					state: "refused",
					operationId,
					reason: error.code,
					...error.limit === void 0 ? {} : { limit: error.limit },
					...error.settingsUrl === void 0 ? {} : { settingsUrl: error.settingsUrl },
					...error.scope === void 0 ? {} : { scope: error.scope }
				};
				if (error instanceof ClientError) throw new ClientError(error.code, error.message, operationId);
				throw error;
			}
		},
		async result(id) {
			input$1(requestId(id));
			return call("result", crypto.randomUUID(), { requestId: id }, (value) => resultState(value, id));
		}
	};
}
//#endregion
//#region src/firebase-bootstrap.ts
/** Hosted public configuration is a corruption-checked carrier, never an access grant. */
var encoder$1 = new TextEncoder();
var decoder$1 = new TextDecoder("utf-8", { fatal: true });
var LINK_BYTES = 8192;
var HOSTED_BOOTSTRAP_MS = 1e4;
/** The origin's own public descriptor grants no space or operations authority. */
async function originLink(origin, fetcher) {
	const controller = new AbortController();
	let timer;
	try {
		valid$5(/^https:\/\/[a-z][a-z0-9-]{4,28}[a-z0-9]\.web\.app$/.test(origin));
		const deadline = new Promise((_, reject) => {
			timer = setTimeout(() => {
				controller.abort();
				reject(new RefusalError("REMOTE_INPUT_INVALID"));
			}, HOSTED_BOOTSTRAP_MS);
		});
		return await Promise.race([deadline, (async () => {
			const row = await readHostedConfig(origin, fetcher, controller.signal);
			const config = parsePublicWebConfig({
				apiKey: row.apiKey,
				appId: row.appId,
				authDomain: row.authDomain,
				projectId: row.projectId
			});
			valid$5(origin === `https://${config.projectId}.web.app`);
			const body = {
				kind: "firestore",
				publicWebConfig: config,
				version: 1
			};
			const descriptor = canonical$1({
				...body,
				checksum: await checksum(body)
			});
			valid$5(!controller.signal.aborted);
			return `${origin}/pair#${base64url(encoder$1.encode(descriptor))}`;
		})()]);
	} catch {
		throw new RefusalError("REMOTE_INPUT_INVALID");
	} finally {
		clearTimeout(timer);
		controller.abort();
	}
}
function valid$5(condition) {
	if (!condition) throw new RefusalError("REMOTE_INPUT_INVALID");
}
function object$2(value, keys) {
	valid$5(typeof value === "object" && value !== null && !Array.isArray(value));
	const record = value;
	valid$5(Object.keys(record).sort().join(",") === [...keys].sort().join(","));
	return record;
}
function parsePublicWebConfig(value) {
	const r = object$2(value, [
		"apiKey",
		"appId",
		"authDomain",
		"projectId"
	]);
	valid$5(typeof r.projectId === "string" && /^[a-z][a-z0-9-]{4,28}[a-z0-9]$/.test(r.projectId));
	valid$5(r.authDomain === `${r.projectId}.firebaseapp.com`);
	for (const key of ["apiKey", "appId"]) valid$5(typeof r[key] === "string" && /^[\x20-\x7e]{1,256}$/.test(r[key]));
	return Object.freeze(r);
}
function canonical$1(value) {
	if (typeof value !== "object" || value === null) return JSON.stringify(value);
	const r = value;
	return `{${Object.keys(r).sort().map((k) => `${JSON.stringify(k)}:${canonical$1(r[k])}`).join(",")}}`;
}
async function checksum(value) {
	return Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", encoder$1.encode(canonical$1(value)))), (b) => b.toString(16).padStart(2, "0")).join("");
}
async function verifyHostedLink(link, origin) {
	try {
		valid$5(typeof link === "string" && encoder$1.encode(link).length <= LINK_BYTES);
		const url = new URL(link);
		valid$5(url.origin === origin && url.protocol === "https:" && url.pathname === "/pair" && url.search === "" && url.username === "" && url.password === "");
		const fragment = url.hash.slice(1);
		valid$5(/^[A-Za-z0-9_-]+$/.test(fragment));
		const bytes = Uint8Array.from(atob(fragment.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
		const text = decoder$1.decode(bytes);
		const r = object$2(JSON.parse(text), [
			"version",
			"kind",
			"publicWebConfig",
			"checksum"
		]);
		valid$5(r.version === 1 && r.kind === "firestore" && typeof r.checksum === "string" && /^[a-f0-9]{64}$/.test(r.checksum));
		const publicConfig = parsePublicWebConfig(r.publicWebConfig);
		valid$5(origin === `https://${publicConfig.projectId}.web.app`);
		valid$5(await checksum({
			kind: r.kind,
			publicWebConfig: publicConfig,
			version: r.version
		}) === r.checksum && text === canonical$1(r));
		valid$5(link === `${origin}/pair#${base64url(bytes)}`);
		return publicConfig;
	} catch {
		throw new RefusalError("REMOTE_INPUT_INVALID");
	}
}
/** A bounded same-origin read cannot follow a redirect into another configuration source. */
async function confirmHostedConfig(expected, origin, fetcher, signal) {
	const r = await readHostedConfig(origin, fetcher, signal);
	for (const key of [
		"projectId",
		"appId",
		"authDomain"
	]) if (r[key] !== expected[key]) throw new ClientError("unverifiable_response", "Hosted configuration does not match this remote link.");
}
/** One bounded response codec for origin-link construction and the unchanged connect check. */
async function readHostedConfig(origin, fetcher, signal) {
	const response = await fetcher(`${origin}/__/firebase/init.json`, {
		signal,
		redirect: "error",
		credentials: "omit",
		cache: "no-store"
	});
	if (!response.ok || response.url !== "" && response.url !== `${origin}/__/firebase/init.json`) throw new ClientError("unverifiable_response", "Hosted configuration could not be confirmed.");
	const reader = response.body?.getReader();
	if (!reader) throw new ClientError("unverifiable_response", "Hosted configuration could not be confirmed.");
	let text = "";
	let size = 0;
	const streamDecoder = new TextDecoder("utf-8", { fatal: true });
	try {
		for (;;) {
			const { done, value } = await reader.read();
			if (signal.aborted) throw new ClientError("timeout", "Hosted configuration check timed out.");
			if (done) break;
			size += value.byteLength;
			if (size > LINK_BYTES) throw new ClientError("unverifiable_response", "Hosted configuration exceeds its bound.");
			text += streamDecoder.decode(value, { stream: true });
		}
		text += streamDecoder.decode();
		const actual = JSON.parse(text);
		if (typeof actual !== "object" || actual === null || Array.isArray(actual)) throw new Error("invalid");
		return actual;
	} finally {
		await reader.cancel();
		reader.releaseLock();
	}
}
//#endregion
//#region src/sealed-sync-mapping.ts
/** Frozen resource field/query mapping. Names are release data, never a Colab-specific list. */
var syncFieldName = (value) => /^[A-Za-z][A-Za-z0-9]{0,63}$/.test(value);
function valid$4(condition) {
	if (!condition) throw new RefusalError("REMOTE_INPUT_INVALID");
}
function row$4(value, keys) {
	valid$4(typeof value === "object" && value !== null && !Array.isArray(value));
	const r = value;
	valid$4(Object.keys(r).sort().join(",") === keys.sort().join(","));
	return r;
}
function parseSyncMapping(value) {
	valid$4(typeof value === "object" && value !== null);
	const r = row$4(value, [
		"fields",
		"spaceField",
		"uidField",
		"query",
		...Object.hasOwn(value, "writeModes") ? ["writeModes"] : []
	]);
	if (r.writeModes !== void 0) valid$4(Array.isArray(r.writeModes) && r.writeModes.length > 0 && new Set(r.writeModes).size === r.writeModes.length && r.writeModes.every((m) => [
		"create",
		"put",
		"delete"
	].includes(m)));
	valid$4(Array.isArray(r.fields) && r.fields.length > 0 && r.fields.length <= 64 && r.fields.every((f) => typeof f === "string" && syncFieldName(f)));
	const fields = new Set(r.fields);
	valid$4(fields.size === r.fields.length);
	const declared = (v) => typeof v === "string" && fields.has(v);
	valid$4(declared(r.spaceField) && (r.uidField === null || declared(r.uidField) && r.uidField !== r.spaceField));
	let query = null;
	if (r.query !== null) {
		valid$4(typeof r.query === "object" && r.query !== null);
		const q = row$4(r.query, [
			"scopeFields",
			"streamField",
			"seqField",
			"order",
			...Object.hasOwn(r.query, "discovery") ? ["discovery"] : []
		]);
		valid$4(Array.isArray(q.scopeFields) && q.scopeFields.length > 0 && q.scopeFields.length <= 8 && q.scopeFields.every(declared));
		const scope = new Set(q.scopeFields);
		valid$4(scope.size === q.scopeFields.length);
		valid$4(declared(q.streamField) && declared(q.seqField) && q.streamField !== q.seqField && q.streamField !== r.spaceField && q.streamField !== r.uidField && q.seqField !== r.uidField && q.order === "asc" && scope.has(r.spaceField) && scope.has(q.streamField) && !scope.has(q.seqField));
		let discovery;
		if (q.discovery !== void 0) {
			const d = row$4(q.discovery, [
				"name",
				"scopeFields",
				"seqEquals"
			]);
			valid$4(typeof d.name === "string" && syncFieldName(d.name) && d.name !== "stream" && Array.isArray(d.scopeFields) && d.scopeFields.every(declared) && new Set(d.scopeFields).size === d.scopeFields.length);
			valid$4([...scope].filter((n) => n !== q.streamField).sort().join(",") === [...d.scopeFields].sort().join(","));
			valid$4(d.seqEquals === null || typeof d.seqEquals === "string" && Number.isSafeInteger(Number(d.seqEquals)) && Number(d.seqEquals) >= 0 && String(Number(d.seqEquals)) === d.seqEquals);
			discovery = Object.freeze({
				name: d.name,
				scopeFields: Object.freeze([...d.scopeFields]),
				seqEquals: d.seqEquals
			});
		}
		query = Object.freeze({
			scopeFields: Object.freeze([...q.scopeFields]),
			streamField: q.streamField,
			seqField: q.seqField,
			order: "asc",
			...discovery ? { discovery } : {}
		});
	}
	return Object.freeze({
		fields: Object.freeze([...fields]),
		...r.writeModes === void 0 ? {} : { writeModes: Object.freeze([...r.writeModes]) },
		spaceField: r.spaceField,
		uidField: r.uidField,
		query
	});
}
//#endregion
//#region src/firebase-declaration.ts
/** Release-supplied data, bound to its exact bytes; no extension-specific resource names. */
var name$1 = /^[a-z][a-z0-9-]{0,31}$/;
var field$2 = /^[A-Za-z][A-Za-z0-9]{0,63}$/;
function require$1(condition) {
	if (!condition) throw new RefusalError("REMOTE_INPUT_INVALID");
}
function members(value, keys) {
	require$1(typeof value === "object" && value !== null && !Array.isArray(value));
	const row = value;
	require$1(Object.keys(row).sort().join(",") === keys.sort().join(","));
	return row;
}
async function verifyHostedDeclaration(input) {
	try {
		require$1(typeof input.bytes === "string" && typeof input.digest === "string");
		const bytes = new TextEncoder().encode(input.bytes);
		require$1(bytes.length <= 65536 && /^[a-f0-9]{64}$/.test(input.digest));
		require$1(Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), (b) => b.toString(16).padStart(2, "0")).join("") === input.digest);
		const value = JSON.parse(input.bytes);
		require$1(typeof value === "object" && value !== null && !Array.isArray(value));
		const row = value;
		members(row, [
			"version",
			"extension",
			"backend",
			"resources",
			"admission",
			...Object.hasOwn(row, "hosting") ? ["hosting"] : []
		]);
		require$1(row.version === 1 && row.backend === "firestore" && typeof row.extension === "string" && name$1.test(row.extension));
		require$1(Array.isArray(row.resources) && row.resources.length <= 64);
		const names = /* @__PURE__ */ new Set();
		for (const value of row.resources) {
			require$1(typeof value === "object" && value !== null);
			const r = members(value, [
				"name",
				"kind",
				"path",
				"limits",
				"ttlField",
				"indexes",
				...Object.hasOwn(value, "sync") ? ["sync"] : []
			]);
			const sync = r.sync === void 0 ? void 0 : parseSyncMapping(r.sync);
			require$1(!["log", "blob"].includes(String(r.kind)) || !sync?.writeModes?.includes("put"));
			require$1(typeof r.name === "string" && name$1.test(r.name) && !names.has(r.name));
			names.add(r.name);
			require$1(typeof r.kind === "string" && [
				"log",
				"checkpoint",
				"blob",
				"awareness"
			].includes(r.kind));
			require$1(typeof r.path === "string");
			const segments = r.path.split("/");
			require$1(segments.length <= 8 && segments.every((s) => /^[a-z0-9_-]{1,64}$/.test(s)));
			require$1(![
				"x",
				"append",
				"subscribe",
				"ack",
				"pair"
			].includes(segments[0]));
			require$1(typeof r.limits === "object" && r.limits !== null);
			const limits = members(r.limits, [
				"maxObjectBytes",
				"maxNamespaceBytes",
				"maxEntries",
				...Object.hasOwn(r.limits, "chunkBytes") ? ["chunkBytes"] : []
			]);
			require$1(Object.values(limits).every((n) => typeof n === "number" && Number.isSafeInteger(n) && n > 0));
			require$1(Number(limits.maxObjectBytes) <= Number(limits.maxNamespaceBytes));
			require$1(limits.chunkBytes === void 0 || Number(limits.chunkBytes) <= Number(limits.maxObjectBytes));
			require$1(r.ttlField === null || typeof r.ttlField === "string" && field$2.test(r.ttlField));
			require$1(Array.isArray(r.indexes) && r.indexes.length <= 64);
			const indexes = /* @__PURE__ */ new Set();
			for (const value of r.indexes) {
				const index = members(value, ["field", "direction"]);
				require$1(typeof index.field === "string" && field$2.test(index.field) && typeof index.direction === "string" && ["asc", "desc"].includes(index.direction));
				const key = `${index.field}:${index.direction}`;
				require$1(!indexes.has(key));
				indexes.add(key);
			}
			require$1(r.kind !== "awareness" || r.ttlField === null && r.indexes.length === 0 && r.sync === void 0);
		}
		const admission = members(row.admission, [
			"artifact",
			"digest",
			"entryPoint"
		]);
		require$1(typeof admission.artifact === "string" && admission.artifact.split("/").length <= 8 && admission.artifact.split("/").every((s) => /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(s)));
		require$1(typeof admission.digest === "string" && /^[a-f0-9]{64}$/.test(admission.digest));
		require$1(typeof admission.entryPoint === "string" && /^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(admission.entryPoint));
	} catch {
		throw new RefusalError("REMOTE_INPUT_INVALID");
	}
}
//#endregion
//#region src/sealed-sync-codec.ts
function valid$3(condition) {
	if (!condition) throw new RefusalError("REMOTE_INPUT_INVALID");
}
function timestamp(s) {
	if (!/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{3}(?:\d{3})?)?Z$/.test(s) || s.startsWith("0000") || s.endsWith("000Z")) return false;
	const base = s.slice(0, 19);
	const date = /* @__PURE__ */ new Date(`${base}Z`);
	return Number.isFinite(date.valueOf()) && date.toISOString().slice(0, 19) === base;
}
function field$1(value) {
	if (typeof value !== "object" || value === null || Array.isArray(value) || Object.keys(value).length !== 1) return false;
	const [kind, v] = Object.entries(value)[0];
	switch (kind) {
		case "stringValue": return typeof v === "string" && ![...v].some((c) => {
			const n = c.codePointAt(0);
			return n >= 55296 && n <= 57343;
		});
		case "integerValue": return typeof v === "string" && Number.isSafeInteger(Number(v)) && String(Number(v)) === v;
		case "booleanValue": return typeof v === "boolean";
		case "nullValue": return v === null;
		case "timestampValue": return typeof v === "string" && timestamp(v);
		case "bytesValue": try {
			return typeof v === "string" && btoa(atob(v)) === v;
		} catch {
			return false;
		}
		default: return false;
	}
}
function canonical(value) {
	if (typeof value !== "object" || value === null || Array.isArray(value)) return JSON.stringify(value);
	return `{${Object.entries(value).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([key, v]) => `${JSON.stringify(key)}:${canonical(v)}`).join(",")}}`;
}
function validateSyncFields(bytes) {
	try {
		const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
		const row = JSON.parse(text);
		valid$3(typeof row === "object" && row !== null && !Array.isArray(row) && canonical(row) === text);
		const fields = row;
		valid$3(Object.keys(fields).length <= 64 && Object.entries(fields).every(([key, v]) => syncFieldName(key) && field$1(v)));
		return fields;
	} catch {
		throw new RefusalError("REMOTE_INPUT_INVALID");
	}
}
function textIs(value, expected) {
	return typeof value === "object" && value !== null && Object.keys(value).length === 1 && value.stringValue === expected;
}
async function validateSyncDocument(context, document, access) {
	const resource = context.resources.find((r) => r.name === document.target.resource);
	valid$3(resource !== void 0 && /^[A-Za-z0-9_+-]{1,256}$/.test(document.target.id) && document.bytes.length <= resource.maxObjectBytes);
	const mapping = resource.sync;
	if (!mapping) throw new RefusalError("REMOTE_STATE_UNAVAILABLE");
	const bytes = new Uint8Array(document.bytes);
	const expectedDigest = document.digest;
	const row = validateSyncFields(bytes);
	valid$3(Object.keys(row).length === mapping.fields.length && mapping.fields.every((key) => Object.hasOwn(row, key)));
	if (mapping.query) {
		const stream = row[mapping.query.streamField];
		const sequence = row[mapping.query.seqField];
		valid$3(typeof stream.stringValue === "string" && /^[A-Za-z0-9_+-]{1,128}$/.test(stream.stringValue) && typeof sequence.integerValue === "string" && Number(sequence.integerValue) > 0);
	}
	valid$3(Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), (b) => b.toString(16).padStart(2, "0")).join("") === expectedDigest);
	if (!textIs(row[mapping.spaceField], context.space) || access === "publish" && mapping.uidField !== null && !textIs(row[mapping.uidField], context.uid)) throw new RefusalError("REMOTE_SCOPE_DENIED");
}
function validateSyncQuery(context, query) {
	const resource = context.resources.find((r) => r.name === query.resource);
	valid$3(resource !== void 0);
	const mapping = resource.sync;
	const spec = mapping?.query;
	if (!mapping || !spec) throw new RefusalError("REMOTE_STATE_UNAVAILABLE");
	valid$3(Object.keys(query.scope).length === spec.scopeFields.length && spec.scopeFields.every((key) => Object.hasOwn(query.scope, key) && field$1(query.scope[key])) && /^[A-Za-z0-9_+-]{1,128}$/.test(query.stream));
	if (!textIs(query.scope[mapping.spaceField], context.space) || !textIs(query.scope[spec.streamField], query.stream)) throw new RefusalError("REMOTE_SCOPE_DENIED");
	valid$3(!query.cursor || query.cursor.stream === query.stream && Number.isSafeInteger(query.cursor.seq) && query.cursor.seq >= 0);
}
function validateSyncDiscovery(context, query) {
	const resource = context.resources.find((r) => r.name === query.resource);
	valid$3(resource !== void 0);
	const mapping = resource.sync;
	const discovery = mapping?.query?.discovery;
	if (!mapping) throw new RefusalError("REMOTE_STATE_UNAVAILABLE");
	if (!discovery) throw Object.assign(new RefusalError("REMOTE_STATE_UNAVAILABLE"), { state: "unsupported" });
	valid$3(query.shape === discovery.name && (query.continuation === void 0 || /^[\x21-\x7e]{1,1024}$/.test(query.continuation)) && Object.keys(query.scope).length === discovery.scopeFields.length && discovery.scopeFields.every((key) => Object.hasOwn(query.scope, key) && field$1(query.scope[key])));
	if (!textIs(query.scope[mapping.spaceField], context.space)) throw new RefusalError("REMOTE_SCOPE_DENIED");
}
async function validateSyncMutation(context, mutation) {
	const target = mutation.mode === "delete" ? mutation.target : mutation.document.target;
	valid$3(/^[A-Za-z0-9_+-]{1,256}$/.test(target.id));
	const resource = context.resources.find((r) => r.name === target.resource);
	valid$3(resource !== void 0);
	const mapping = resource.sync;
	if (!mapping || !(mapping.writeModes ?? ["create"]).includes(mutation.mode)) throw Object.assign(new RefusalError("REMOTE_STATE_UNAVAILABLE"), { state: "unsupported" });
	if (mutation.mode !== "delete") await validateSyncDocument(context, mutation.document, "publish");
}
/** Page-scope registration, not a stream query or mandatory discovery shape. */
function validateSyncInterest(context, interest) {
	const resource = context.resources.find((r) => r.name === interest.resource);
	valid$3(resource !== void 0);
	const mapping = resource.sync;
	const query = mapping?.query;
	if (!mapping || !query) throw new RefusalError("REMOTE_STATE_UNAVAILABLE");
	const fields = query.scopeFields.filter((key) => key !== query.streamField);
	valid$3(Object.keys(interest.scope).length === fields.length && fields.every((key) => Object.hasOwn(interest.scope, key) && field$1(interest.scope[key])));
	if (!textIs(interest.scope[mapping.spaceField], context.space)) throw new RefusalError("REMOTE_SCOPE_DENIED");
}
//#endregion
//#region src/sealed-sync.ts
/** Extension-scoped sealed-document ports and pure binding. No credential storage. */
var identity = /^[A-Za-z0-9_+-]{1,128}$/;
var documentId = /^[A-Za-z0-9_+-]{1,256}$/;
function valid$2(condition) {
	if (!condition) throw new RefusalError("REMOTE_INPUT_INVALID");
}
/** Caller supplies bootstrap-verified public config. This pure binding does not attest an origin. */
async function bindSyncContext(config, declaration, space, auth) {
	config = parsePublicWebConfig(config);
	await verifyHostedDeclaration(declaration);
	valid$2(identity.test(space) && identity.test(auth.identity.uid));
	if (config.projectId !== auth.identity.projectId) throw new RefusalError("REMOTE_SCOPE_DENIED");
	const row = JSON.parse(declaration.bytes);
	for (const [i, resource] of row.resources.entries()) {
		const segments = resource.path.split("/").length;
		valid$2(segments <= 7 && segments % 2 === 1);
		valid$2(!row.resources.slice(0, i).some((other) => resource.path === other.path || resource.path.startsWith(`${other.path}/`) || other.path.startsWith(`${resource.path}/`)));
	}
	const resources = row.resources.map((r) => Object.freeze({
		name: r.name,
		path: r.path,
		maxObjectBytes: r.limits.maxObjectBytes,
		...r.sync === void 0 ? {} : { sync: parseSyncMapping(r.sync) },
		...r.limits.chunkBytes === void 0 ? {} : { chunkBytes: r.limits.chunkBytes }
	}));
	return Object.freeze({
		config: Object.freeze({ ...config }),
		extension: row.extension,
		declarationDigest: declaration.digest,
		space,
		uid: auth.identity.uid,
		resources: Object.freeze(resources),
		auth
	});
}
/** A document target cannot escape its declared collection or select an operations route. */
function syncDocumentPath(context, target) {
	const resource = context.resources.find((r) => r.name === target.resource);
	valid$2(resource !== void 0 && documentId.test(target.id));
	return `x/${context.extension}/${resource.path}/${target.id}`;
}
/** Pure complete-or-absent validation; never returns a truncated object. */
async function verifySealedObject(original, manifest, chunks, maxObjectBytes) {
	valid$2(documentId.test(original) && manifest.original === original);
	valid$2(Number.isSafeInteger(maxObjectBytes) && maxObjectBytes > 0 && maxObjectBytes <= 16779264);
	valid$2(Number.isSafeInteger(manifest.chunkBytes) && manifest.chunkBytes > 0 && manifest.chunkBytes <= maxObjectBytes);
	valid$2(Number.isSafeInteger(manifest.bytes) && manifest.bytes >= 0 && manifest.bytes <= maxObjectBytes);
	valid$2(Number.isSafeInteger(manifest.count) && manifest.count >= 0 && manifest.count <= 513 && manifest.count === Math.ceil(manifest.bytes / manifest.chunkBytes));
	valid$2(chunks.length === manifest.count && /^[a-f0-9]{64}$/.test(manifest.digest));
	const bytes = new Uint8Array(manifest.bytes);
	let offset = 0;
	for (let i = 0; i < chunks.length; i++) {
		const chunk = chunks[i];
		valid$2(chunk.byteLength === Math.min(manifest.chunkBytes, manifest.bytes - offset));
		bytes.set(chunk, offset);
		offset += chunk.byteLength;
	}
	valid$2(Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), (b) => b.toString(16).padStart(2, "0")).join("") === manifest.digest);
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
//#region src/remote-shell.ts
var current;
/** The mounted route's part of the one shell, or undefined outside the app (unit tests). */
function remoteShell() {
	return current;
}
/** Give Remote's page code the mounted route's slot and lifetime; `undefined` when it unmounts. */
function hostRemote(slot, signal) {
	current = slot && signal && {
		setMenu: (menu, select) => slot.setMenu(menu, select),
		describe: (address) => slot.describe(address),
		described: (listener) => slot.described(listener),
		signal
	};
}
/** A shared list row (`tmt-ui-list*`): mark, linked title, then its detail as metadata. */
function row$3(href, mark, title, detail) {
	const item = document.createElement("li");
	item.className = "tmt-ui-list-row";
	item.dataset.leading = "";
	const sign = document.createElement("div");
	sign.className = "tmt-ui-list-leading";
	sign.setAttribute("aria-hidden", "true");
	sign.textContent = mark;
	if (mark === "◆") sign.dataset.tone = "waiting";
	const name = document.createElement("div");
	name.className = "tmt-ui-list-title";
	const link = document.createElement("a");
	link.href = href;
	link.textContent = title;
	name.append(link);
	const meta = document.createElement("div");
	meta.className = "tmt-ui-list-meta";
	const more = document.createElement("div");
	more.className = "tmt-ui-list-updated";
	more.textContent = detail;
	meta.append(more);
	item.append(sign, name, meta);
	return item;
}
/** One Needs-you row per module with a count and a waiting pairing offer; no per-item list until
* modules publish items. Recent pages are the modules' described home cards. `open` turns a
* module's described path into where this app opens it. */
function homePage(shell, open) {
	const needs = document.getElementById("home-needs");
	const note = document.getElementById("home-needs-note");
	const recent = document.getElementById("home-recent");
	const recentBlock = document.getElementById("home-recent-block");
	if (!needs || !note || !recent || !recentBlock) return;
	const offer = fetch("/sdk/pair-offer", { cache: "no-store" }).then((response) => response.status === 200).catch(() => false);
	shell.described(async (answer) => {
		const rows = [];
		for (const module of answer?.modules ?? []) if (module.needsYou) rows.push(row$3(open(module.rail.path), "◆", `${module.needsYou} in ${module.rail.label}`, "Needs you"));
		if (await offer) rows.push(row$3(new URL("/remote/pair", location.href).href, "◆", "A device is waiting to pair", "Remote · confirm the four words in the terminal"));
		needs.replaceChildren(...rows);
		if (answer) note.textContent = rows.length ? "" : "Nothing needs you.";
		else {
			const pair = Object.assign(document.createElement("a"), {
				href: "/remote/",
				textContent: "Pair this browser"
			});
			note.replaceChildren(pair, " in Remote to see what needs you.");
		}
		note.hidden = !note.textContent;
		const cards = (answer?.modules ?? []).flatMap((module) => (module.home ?? []).map((card) => row$3(open(card.path), "▤", card.title, module.rail.label)));
		recent.replaceChildren(...cards);
		recentBlock.hidden = cards.length === 0;
	});
}
//#endregion
//#region src/remote-state.ts
/** Current-origin observation only. A foreign entry has no mark or live-access claim. A connection
* state is words only, in the neutral tone: the shared agent marks mean agent states, not these. */
function remoteState(state, current = true) {
	if (!current) return {
		text: "not checked",
		word: "not checked",
		mark: "",
		tone: "muted",
		action: ""
	};
	const [text, word, action] = state === "Connected" ? [
		"Connected",
		"connected",
		""
	] : state === "Checking" ? [
		"Checking…",
		"connecting",
		""
	] : state === "Access ended" ? [
		"Access ended",
		"access ended",
		"Pair again"
	] : state === "Different machine" ? [
		"Different machine",
		"different machine",
		"Pair again"
	] : state === "Not paired" || state === "Pairing unreadable" ? [
		state,
		"not paired",
		"Pair again"
	] : [
		"Unconfirmed",
		"unreachable",
		"Retry"
	];
	return {
		text,
		word,
		mark: "",
		tone: "neutral",
		action
	};
}
/** Both page owners and the menu identify the current Local row by the same door root. */
function currentRemote(entry, address, pageUrl) {
	const root = new URL("./", pageUrl);
	if (root.pathname === "/remote/") root.pathname = "/";
	return entry.kind === "local" && (entry.url === address || new URL(entry.url).href === root.href);
}
//#endregion
//#region src/remote-menu.ts
/**
* The menu rows that open a part of a page. Remote's pages treat any URL fragment as untrusted
* input and erase it, so a part is never addressed by `#id`: the menu remembers the chosen part
* for this tab and scrolls to it, on this page or once the other page mounts.
*/
var PARTS = {
	devices: {
		page: "settings",
		target: "devices-title"
	},
	machine: {
		page: "settings",
		target: "opening-title"
	},
	pair: {
		page: "landing",
		target: "heading"
	}
};
var PART_KEY = "tmt.remote.menu.part";
function chosenPart() {
	try {
		const value = sessionStorage.getItem(PART_KEY);
		return value && value in PARTS ? value : void 0;
	} catch {
		return;
	}
}
function choosePart(part) {
	try {
		if (part) sessionStorage.setItem(PART_KEY, part);
		else sessionStorage.removeItem(PART_KEY);
	} catch {}
}
/** `shell` is the mounted route's part of the app; a page bundled apart (settings) is handed it. */
function showPart(part) {
	document.getElementById(PARTS[part].target)?.scrollIntoView({ block: "start" });
}
function remoteMenu(actions, shell = remoteShell()) {
	if (!shell) return () => {};
	const page = document.documentElement.dataset.tmtPage;
	const landing = new URL("/remote/", location.href).href;
	const settings = new URL("/remote/settings", location.href).href;
	let catalog = {
		version: 1,
		entries: []
	};
	let state = "Checking";
	let address = "";
	const run = /* @__PURE__ */ new Map();
	const chosen = chosenPart();
	choosePart(void 0);
	let shownPart = chosen && PARTS[chosen].page === page ? chosen : void 0;
	function paint() {
		run.clear();
		const view = remoteState(state);
		const reason = actions.blocked();
		const selected = catalog.entries.find((entry) => currentRemote(entry, address, location.href));
		const entries = [...catalog.entries].sort((a, b) => Number(a.kind === "firestore") - Number(b.kind === "firestore"));
		const remotes = [];
		const onSettings = page === "settings";
		const here = shownPart;
		const lit = (id) => here === id;
		const onLanding = !onSettings && !here;
		if (!selected) remotes.push({
			id: "current",
			label: [view.mark, `This machine · ${view.word}`].filter(Boolean).join(" "),
			href: landing,
			current: onLanding
		});
		for (const entry of entries) {
			const here = currentRemote(entry, address, location.href);
			const shown = remoteState(state, here);
			const id = `remote:${entry.name}`;
			if (!here) run.set(id, () => actions.open(entry));
			remotes.push({
				id,
				label: here ? [shown.mark, `${entry.name} · ${shown.word}`].filter(Boolean).join(" ") : `Open ${entry.name} · ${entry.kind === "local" ? "Local" : `Firebase · ${entry.project}`} · not checked`,
				href: here ? landing : "#",
				current: here && onLanding,
				disabled: !here && !!reason,
				note: !here && reason ? reason : void 0
			});
		}
		if (view.action) {
			run.set("recover", () => actions.recover(remoteState(state).action));
			remotes.push({
				id: "recover",
				label: view.action,
				href: "#",
				current: false,
				note: view.text
			});
		}
		run.set("add", actions.add);
		shell.setMenu({
			title: "Remote",
			groups: [
				{ items: remotes },
				{
					label: "This machine",
					items: [
						{
							id: "devices",
							label: "Devices",
							href: settings,
							current: lit("devices")
						},
						{
							id: "pair",
							label: "Connect a device",
							href: landing,
							current: lit("pair")
						},
						{
							id: "machine",
							label: "This machine’s settings",
							href: settings,
							current: lit("machine")
						}
					]
				},
				{ items: [{
					id: "add",
					label: "Add remote…",
					href: `${settings}#add-remote`,
					current: false
				}, {
					id: "settings",
					label: "Remote settings",
					href: settings,
					current: onSettings && !here
				}] }
			]
		}, (id) => {
			if (id in PARTS) {
				const part = id;
				if (PARTS[part].page !== page) return choosePart(part), false;
				shownPart = part;
				showPart(part);
				paint();
				return true;
			}
			if (id === "settings" || id === "current") {
				if (id === "settings" === onSettings) {
					shownPart = void 0;
					window.scrollTo({ top: 0 });
					paint();
					return true;
				}
				return false;
			}
			const action = run.get(id);
			if (!action) return false;
			if (!id.startsWith("remote:") || !actions.blocked()) action();
			else paint();
			return true;
		});
	}
	paint();
	if (shownPart) {
		const opening = shownPart;
		requestAnimationFrame(() => showPart(opening));
	}
	const refresh = () => {
		paint();
		actions.refresh();
	};
	const entered = (event) => {
		const menu = event.target?.closest?.(".tmt-ui-module-menu");
		if (menu && !menu.contains(event.relatedTarget)) refresh();
	};
	document.addEventListener("pointerover", entered, { signal: shell.signal });
	document.addEventListener("focusin", entered, { signal: shell.signal });
	if (page === "settings") shell.describe();
	return (next, observed, checkedAddress) => {
		catalog = next;
		state = observed;
		address = checkedAddress;
		if (observed === "Connected" && checkedAddress) shell.describe(checkedAddress);
		paint();
	};
}
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
var SVG$2 = "http://www.w3.org/2000/svg";
/** The busy mark's SVG (its dots in currentColor), as `BusyMark` renders it. */
function busySvg(doc) {
	const svg = doc.createElementNS(SVG$2, "svg");
	svg.setAttribute("viewBox", "0 0 16 16");
	for (const [cx, cy] of busyDots) {
		const dot = doc.createElementNS(SVG$2, "circle");
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
function valid$1(condition, message = "The remote name or link is invalid") {
	if (!condition) throw new RemoteRegistryError("REMOTE_SELECTOR_INVALID", message);
}
function fields(value, keys) {
	valid$1(typeof value === "object" && value !== null && !Array.isArray(value));
	const object = value;
	valid$1(Object.keys(object).sort().join(",") === [...keys].sort().join(","));
	return object;
}
function name(value) {
	valid$1(typeof value === "string" && /^[a-z0-9-]{1,63}$/.test(value) && value !== "local");
}
function target(entry) {
	return new URL(entry.url).href;
}
async function reference(alias, address) {
	name(alias);
	valid$1(typeof address === "string" && address.length <= LIMIT);
	let url;
	try {
		url = new URL(address);
	} catch {
		valid$1(false);
	}
	if (url.protocol === "https:" && url.hostname.endsWith(".web.app") && url.pathname === "/pair") return {
		name: alias,
		kind: "firestore",
		url: address,
		project: (await verifyHostedLink(address, url.origin).catch(() => {
			valid$1(false);
		})).projectId
	};
	valid$1(address.length <= 2048 && /^https?:\/\/[A-Za-z0-9.:-]+\/[A-Za-z0-9/_-]*$/.test(address) && address.endsWith("/") && !url.search && !url.hash && !url.username && !url.password);
	return {
		name: alias,
		kind: "local",
		url: address,
		project: null
	};
}
async function parseCatalog(value) {
	const row = fields(value, ["version", "entries"]);
	valid$1(row.version === 1 && Array.isArray(row.entries));
	valid$1(row.entries.length <= 16, "This browser can store at most 16 remotes. Remove one before adding another.");
	const entries = [];
	for (const raw of row.entries) {
		const r = fields(raw, [
			"name",
			"kind",
			"url",
			"project"
		]);
		const entry = await reference(r.name, r.url);
		valid$1(entry.kind === r.kind && entry.project === r.project);
		valid$1(!entries.some((old) => old.name === entry.name || target(old) === target(entry)));
		entries.push(Object.freeze(entry));
	}
	const catalog = {
		version: 1,
		entries: Object.freeze(entries)
	};
	valid$1(encoder.encode(json(catalog)).length <= LIMIT, "The remote list is too large to store in this browser. Remove one before adding another.");
	return Object.freeze(catalog);
}
function json(catalog) {
	return JSON.stringify(catalog);
}
function decode$1(value) {
	valid$1(/^[A-Za-z0-9_-]+$/.test(value));
	try {
		const bytes = Uint8Array.from(atob(value.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
		valid$1(base64url(bytes) === value);
		return JSON.parse(decoder.decode(bytes));
	} catch {
		valid$1(false);
	}
}
function hint(value) {
	const r = fields(value, ["spaceId", "pageId"]);
	valid$1(typeof r.spaceId === "string" && /^[a-z2-7]{32}$/.test(r.spaceId));
	valid$1(typeof r.pageId === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(r.pageId));
	return {
		spaceId: r.spaceId,
		pageId: r.pageId
	};
}
async function parseHandoff(fragment) {
	valid$1(fragment.startsWith("#") && encoder.encode(fragment).length <= LIMIT);
	const params = new URLSearchParams(fragment.slice(1));
	const keys = [...params.keys()];
	valid$1(keys.length === new Set(keys).size && keys.every((key) => ["tmt-remotes", "tmt-page"].includes(key)));
	const raw = params.get("tmt-remotes");
	valid$1(raw !== null);
	const catalog = await parseCatalog(decode$1(raw));
	valid$1(base64url(encoder.encode(json(catalog))) === raw);
	const page = params.get("tmt-page");
	const checkedHint = page === null ? void 0 : hint(decode$1(page));
	if (checkedHint) valid$1(base64url(encoder.encode(JSON.stringify(checkedHint))) === page);
	return {
		catalog,
		...checkedHint ? { hint: checkedHint } : {}
	};
}
async function buildHandoff(destination, value, page) {
	const checked = await reference(destination.name, destination.url);
	valid$1(checked.kind === destination.kind && checked.project === destination.project);
	if (checked.kind === "firestore") return checked.url;
	const catalog = await parseCatalog(value);
	const params = new URLSearchParams({ "tmt-remotes": base64url(encoder.encode(json(catalog))) });
	if (page !== void 0) params.set("tmt-page", base64url(encoder.encode(JSON.stringify(hint(page)))));
	valid$1(encoder.encode(`#${params}`).length <= LIMIT, "The remote list is too large to send along when opening this remote. Remove an entry, then try again.");
	return `${new URL("/remote/", checked.url).href}#${params}`;
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
	valid$1(choices.length === checked.rows.length);
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
		valid$1(raw === void 0 || typeof raw === "string" && raw.length <= LIMIT && encoder.encode(raw).length <= LIMIT);
		const catalog = raw === void 0 ? empty : await parseCatalog(JSON.parse(raw));
		valid$1(raw === void 0 || raw === json(catalog));
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
		valid$1(plan.rows[0].status !== "duplicate", `Already added as ${plan.rows[0].existingName}`);
		await remoteRegistry.confirmImport(plan, [alias]);
	},
	async rename(previous, alias) {
		name(alias);
		const latest = await snapshot();
		valid$1(latest.catalog.entries.some((entry) => entry.name === previous));
		valid$1(!latest.catalog.entries.some((entry) => entry.name === alias && entry.name !== previous), `You already have a remote named ${alias}`);
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
		valid$1(latest.catalog.entries.some((entry) => entry.name === alias));
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
function node(tag, text = "") {
	const result = document.createElement(tag);
	result.textContent = text;
	return result;
}
function action(text, run) {
	const button = node("button", text);
	button.type = "button";
	button.className = "tmt-ui-action";
	button.addEventListener("click", run);
	return button;
}
var fieldCount = 0;
function field(label, value) {
	const wrapper = node("div");
	wrapper.className = browserUiClasses.field;
	const input = node("input");
	input.id = `remote-field-${++fieldCount}`;
	input.className = browserUiClasses.fieldControl;
	input.value = value;
	const text = node("label", label);
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
function remoteRegistryPage(blocked = () => "", shell) {
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
	}, shell);
	const showError = (error, slot) => {
		slot.textContent = error instanceof RemoteRegistryError ? error.message : "The remotes could not be read";
	};
	const dialog = (title, opener) => {
		currentDialog?.close();
		const box = node("dialog");
		box.className = browserUiClasses.dialog;
		const header = node("header");
		header.className = browserUiClasses.dialogHeader;
		const heading = node("h2", title);
		heading.id = "remote-dialog-heading";
		header.append(heading);
		box.setAttribute("aria-labelledby", heading.id);
		const content = node("div");
		content.className = browserUiClasses.dialogBody;
		const body = node("div");
		const message = node("p");
		message.setAttribute("role", "status");
		content.append(body, message);
		const buttons = node("footer");
		buttons.className = browserUiClasses.dialogActions;
		box.append(header, content, buttons);
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
		view.body.append(node("p", "Only names and addresses are copied. No keys, pairings or drafts."));
		const choices = [];
		for (const row of plan.rows) {
			const item = node("section");
			item.className = browserUiClasses.section;
			const name = node("h3", row.entry.name);
			name.className = browserUiClasses.sectionHeading;
			name.dataset.variant = "title";
			item.append(name, node("p", `${kindLabel(row.entry)} · ${row.entry.url}`));
			if (row.status === "duplicate") {
				item.append(node("p", `Already added as ${row.existingName}`));
				choices.push(() => null);
			} else if (row.status === "collision") {
				item.append(node("p", `You already have a remote named ${row.entry.name}`));
				const alias = field("Remote name", row.suggestedName);
				const skip = node("input");
				skip.type = "checkbox";
				skip.className = browserUiClasses.checkbox;
				const skipLabel = node("label", "Skip");
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
		view.body.append(node("p", entry.kind === "firestore" ? `${entry.project}.web.app` : entry.url));
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
		view.body.append(node("p", "This only removes the entry here. It does not sign out or revoke access."));
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
				const row = node("li");
				row.className = browserUiClasses.listRow;
				row.dataset.remote = entry.name;
				row.dataset.url = entry.url;
				row.dataset.kind = entry.kind;
				const title = node("div");
				title.className = browserUiClasses.listTitle;
				const label = node("span", entry.name);
				label.className = browserUiClasses.listLabel;
				const description = node("span", `${kindLabel(entry)} · ${entry.url}`);
				description.className = browserUiClasses.listDescription;
				title.append(label, description);
				const meta = node("div");
				meta.className = browserUiClasses.listMeta;
				const state = node("div", remoteState(observed, current(entry)).text);
				state.className = browserUiClasses.listState;
				state.dataset.tone = remoteState(observed, current(entry)).tone;
				meta.append(state);
				const actions = node("div");
				actions.className = browserUiClasses.listActions;
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
				row.append(title, meta, actions);
				return row;
			});
			list.replaceChildren(...rows);
			if (!rows.length) {
				const empty = node("li");
				empty.className = browserUiClasses.listRow;
				const text = node("div", "No remotes added in this browser.");
				text.className = browserUiClasses.listTitle;
				empty.append(text);
				list.append(empty);
			}
			menu(catalog, observed, address ?? "");
		} catch (error) {
			showError(error, status);
		}
	};
	add.addEventListener("click", () => {
		const view = dialog("Add remote", add);
		const alias = field("Remote name", "");
		const url = field("Remote address or link", "");
		view.body.append(alias.label, url.label);
		view.buttons.append(action("Continue", () => {
			reference(alias.input.value, url.input.value).then((entry) => {
				if (currentDialog !== view.box || !view.box.open || !view.box.isConnected) return;
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
	}, { signal: (shell ?? remoteShell())?.signal });
	return (state, checkedAddress) => {
		observed = state;
		if (checkedAddress) address = checkedAddress;
		for (const row of list.querySelectorAll("[data-remote]")) {
			const stateNode = row.querySelector(".tmt-ui-list-state");
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
//#region ../../../../design/browser-ui/src/brand-mark-svg.ts
/** A polygon whose corners are quadratic curves of the given radii (0 keeps a corner sharp). */
function rounded(points, radii) {
	const n = points.length;
	const toward = ([ax, ay], [bx, by], k) => {
		const length = Math.hypot(bx - ax, by - ay);
		const t = Math.min(k, length / 2) / length;
		return [ax + (bx - ax) * t, ay + (by - ay) * t];
	};
	let d = "";
	for (let i = 0; i < n; i++) {
		const corner = points[i];
		const [sx, sy] = toward(corner, points[(i - 1 + n) % n], radii[i]);
		const [ex, ey] = toward(corner, points[(i + 1) % n], radii[i]);
		d += `${i ? "L" : "M"}${sx.toFixed(2)} ${sy.toFixed(2)}Q${corner[0]} ${corner[1]} `;
		d += `${ex.toFixed(2)} ${ey.toFixed(2)}`;
	}
	return `${d}Z`;
}
var FULL = {
	panel: rounded([
		[6, 10],
		[50, 24],
		[50, 80],
		[6, 88]
	], [
		8,
		6.5,
		6.5,
		8
	]),
	beam: rounded([
		[7, 10],
		[74, 22],
		[80, 28],
		[80, 80],
		[74, 86],
		[50, 80],
		[7, 70]
	], [
		0,
		7,
		7,
		7,
		7,
		0,
		0
	]),
	rim: "M49.4 29 L49.4 75",
	prompt: "M15.5 34 L23.5 40.5 L15.5 47 M26.5 49.5 L34 51.2"
};
var SMALL = {
	panel: rounded([
		[5, 14],
		[52, 24],
		[52, 76],
		[5, 86]
	], [
		7,
		7,
		7,
		7
	]),
	beam: rounded([
		[6, 14],
		[86, 24],
		[92, 29],
		[92, 81],
		[86, 86],
		[52, 76],
		[6, 70]
	], [
		0,
		9,
		9,
		9,
		9,
		0,
		0
	]),
	prompt: "M15 37 L28 50 L15 63"
};
var brandMarkSmall = (size) => size !== void 0 && size < 28;
var stop = (name, offset, opacity) => `<stop class="tmt-brand-mark-${name}" offset="${offset}"${opacity === void 0 ? "" : ` stop-opacity="${opacity}"`}></stop>`;
var light = (offset, opacity) => `<stop offset="${offset}" stop-color="#fff" stop-opacity="${opacity}"></stop>`;
var box = "filterUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"96\" height=\"96\"";
/** The mark's children; `id` prefixes its paint servers, unique per document. */
function brandMarkInner(small, id) {
	const shape = small ? SMALL : FULL;
	const aurora = `<linearGradient id="${id}-aurora" x1="0.2" y1="0.05" x2="0.85" y2="1">${stop("cyan", 0)}${stop("green", .5)}${stop("gold", 1)}</linearGradient><linearGradient id="${id}-far" x1="0" y1="0" x2="1" y2="0">${stop("far", .45, 0)}${stop("far", 1, small ? .15 : .25)}</linearGradient><linearGradient id="${id}-panel" x1="0" y1="0" x2="0.6" y2="1">${stop("panel-top", 0)}${stop("panel-bottom", 1)}</linearGradient><linearGradient id="${id}-mono" x1="0" y1="0" x2="1" y2="0.35">${stop("mono-start", 0)}${stop("mono-end", 1)}</linearGradient>`;
	const beam = `<path d="${shape.beam}" fill="url(#${id}-aurora)"></path><path d="${shape.beam}" fill="url(#${id}-far)"></path>`;
	const print = `<path class="tmt-brand-mark-print" d="${shape.beam}" fill="url(#${id}-mono)"></path>`;
	const panel = `<path class="tmt-brand-mark-panel" d="${shape.panel}" fill="url(#${id}-panel)"></path>`;
	const prompt = `<path class="tmt-brand-mark-prompt" d="${shape.prompt}"></path>`;
	if (small) return `<defs>${aurora}</defs><g class="tmt-brand-mark-beam">${beam}</g>${print}${panel}${prompt}`;
	return `<defs>${aurora}<linearGradient id="${id}-halo-weight" x1="0" y1="0" x2="1" y2="1">${light(.3, .15)}${light(1, 1)}</linearGradient><mask id="${id}-halo-mask" maskUnits="userSpaceOnUse" x="0" y="0" width="96" height="96"><rect width="96" height="96" fill="url(#${id}-halo-weight)"></rect></mask><linearGradient id="${id}-feather" x1="0" y1="0" x2="1" y2="0">${light(.88, 1)}${light(1, .7)}</linearGradient><mask id="${id}-feather-mask" maskContentUnits="objectBoundingBox"><rect width="1" height="1" fill="url(#${id}-feather)"></rect></mask><linearGradient id="${id}-spill" gradientUnits="userSpaceOnUse" x1="50" y1="0" x2="36" y2="0">${stop("glow", 0, .6)}${stop("cyan", .35, .22)}${stop("cyan", 1, 0)}</linearGradient><linearGradient id="${id}-sheen" x1="0" y1="0" x2="1" y2="1">${light(0, .1)}${light(.35, 0)}</linearGradient><linearGradient id="${id}-rim" x1="0" y1="0" x2="0" y2="1">${stop("cyan", 0, .9)}${stop("gold", 1, .9)}</linearGradient><clipPath id="${id}-panel-clip"><path d="${shape.panel}"></path></clipPath><filter id="${id}-halo" ${box}><feGaussianBlur stdDeviation="2.4"></feGaussianBlur></filter><filter id="${id}-soft" ${box}><feGaussianBlur stdDeviation="0.25"></feGaussianBlur></filter></defs><g transform="translate(6.5 3) scale(0.94)"><g class="tmt-brand-mark-beam"><g mask="url(#${id}-halo-mask)"><path d="${shape.beam}" fill="url(#${id}-aurora)" filter="url(#${id}-halo)" opacity="0.55" transform="translate(1 1.4)"></path></g><g mask="url(#${id}-feather-mask)" filter="url(#${id}-soft)">${beam}</g></g>` + print + panel + `<g class="tmt-brand-mark-beam" clip-path="url(#${id}-panel-clip)"><path d="${shape.panel}" fill="url(#${id}-spill)"></path><path class="tmt-brand-mark-rim" d="${FULL.rim}" stroke="url(#${id}-rim)"></path></g><path d="${shape.panel}" fill="url(#${id}-sheen)"></path>` + prompt + `</g>`;
}
/** The whole decorative `<svg>`, as static hosts embed it. */
function brandMarkSvg({ size, className = "tmt-brand-mark", id }) {
	const small = brandMarkSmall(size);
	const dimensions = size === void 0 ? "" : ` width="${size}" height="${size}"`;
	return `<svg class="${className}" data-form="${small ? "small" : "full"}" viewBox="0 0 96 96" aria-hidden="true"${dimensions}>${brandMarkInner(small, id)}</svg>`;
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
//#endregion
//#region ../../../../design/browser-ui/src/shell-icons.ts
/**
* Lucide 1.52.0 (ISC, see `LICENSE.lucide`) node data for the shell, so static hosts draw the
* same icons without a React or Lucide runtime. The first eight are the module icon allow-list
* of Remote's describe contract; an unknown name falls back to `box`.
*/
var shellIconNodes = {
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
	"panel-left-close": [
		["rect", {
			width: "18",
			height: "18",
			x: "3",
			y: "3",
			rx: "2"
		}],
		["path", { d: "M9 3v18" }],
		["path", { d: "m16 15-3-3 3-3" }]
	],
	"panel-left-open": [
		["rect", {
			width: "18",
			height: "18",
			x: "3",
			y: "3",
			rx: "2"
		}],
		["path", { d: "M9 3v18" }],
		["path", { d: "m14 9 3 3-3 3" }]
	],
	menu: [
		["path", { d: "M4 5h16" }],
		["path", { d: "M4 12h16" }],
		["path", { d: "M4 19h16" }]
	]
};
var moduleIcons = [
	"house",
	"file-text",
	"monitor-smartphone",
	"users",
	"chart-column",
	"search",
	"settings",
	"box"
];
var moduleIcon = (name) => moduleIcons.includes(name) ? name : "box";
//#endregion
//#region ../../../../design/browser-ui/src/shell.ts
/** Counts above 99 read `99+`; needs-you counts carry the ◆ mark and colour. */
function shellCount(value, kind = "needs-you") {
	return {
		text: value > 99 ? "99+" : String(value),
		needsYou: kind === "needs-you"
	};
}
/** The answer this browser read last, shared by every page so each paints the same rail first. */
var ANSWER_KEY = "tmt-shell-answer-v1";
function rememberedShellAnswer() {
	try {
		const stored = JSON.parse(sessionStorage.getItem(ANSWER_KEY) ?? "null");
		return stored?.version === 1 && Array.isArray(stored.modules) ? stored : void 0;
	} catch {
		return;
	}
}
/** The door's mount of one module: `/r/<prefix>/x/<module>/`, the prefix being the machine's 16
* lowercase base32 letters. The one definition of that shape for every product. */
/** A module mount, `/r/<prefix>/x/<module>/`, at the start of a path. */
var MODULE_MOUNT = /^(\/r\/[a-z2-7]{16})\/x\/([a-z0-9][a-z0-9-]{0,31})\//;
/** The module a mount path names, or undefined when `pathname` is not exactly a module mount. */
function moduleMount(pathname) {
	const mount = MODULE_MOUNT.exec(pathname);
	return mount?.[0] === pathname ? mount[2] : void 0;
}
/** The module whose mount `pathname` lies under, and the rest of the path below that mount. */
function underModuleMount(pathname) {
	const mount = MODULE_MOUNT.exec(pathname);
	return mount ? {
		module: mount[2],
		rest: pathname.slice(mount[0].length)
	} : void 0;
}
/**
* Where this page reads the composed shell: the `/r/<prefix>` mount it sits under, else the door's
* answer for its path (`POST /sdk/mount`). Undefined when neither answers.
*/
async function shellAddress() {
	const mounted = MODULE_MOUNT.exec(location.pathname);
	if (mounted) return `${location.origin}${mounted[1]}`;
	try {
		const response = await fetch("/sdk/mount", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ path: location.pathname })
		});
		if (response.status !== 200) return void 0;
		const { address } = await response.json();
		return typeof address === "string" ? address : void 0;
	} catch {
		return;
	}
}
/** Fetch the owner's composed shell once per page; any failure leaves the remembered answer. */
async function loadShellAnswer(address) {
	try {
		const response = await fetch(`${address}/x/-/shell`, {
			credentials: "same-origin",
			cache: "no-store"
		});
		if (!response.ok) return void 0;
		const answer = await response.json();
		if (answer?.version !== 1 || !Array.isArray(answer.modules)) return void 0;
		try {
			sessionStorage.setItem(ANSWER_KEY, JSON.stringify(answer));
		} catch {}
		return answer;
	} catch {
		return;
	}
}
/**
* The one rail (spec c5ccd17a): the same items in the same order on every page, whatever module
* hosts it. A module the composed answer does not describe stays, dimmed, with the reason.
*/
var RAIL = [
	{
		id: "colab",
		label: "Colab",
		icon: "file-text",
		path: "/colab/"
	},
	{
		id: "remote",
		label: "Remote",
		icon: "monitor-smartphone",
		path: "/remote/"
	},
	{
		id: "ops",
		label: "Ops",
		icon: "users",
		path: "/ops/"
	},
	{
		id: "digest",
		label: "Digest",
		icon: "chart-column",
		path: "/digest/"
	}
];
/** Served by the TMT host itself, so always reachable from the rail. */
var HOST_SERVED = /* @__PURE__ */ new Set([
	"remote",
	"ops",
	"digest"
]);
/**
* Home, the fixed modules (Colab, Remote, Ops, Digest), any other described module in Remote's
* order, then TMT settings at the bottom. Colab is available
* when the answer describes it or it hosts the page; without an answer (not paired) it is dimmed.
* The current item is the host's module, else the longest matching path.
*/
function shellRail(answer, place, host) {
	const active = host?.extension;
	const described = new Map((answer?.modules ?? []).map((module) => [module.extension, module]));
	const hosted = (id) => host?.extension === id;
	const path = place.href.slice(place.origin.length);
	const settings = path === "/settings" || path.startsWith("/settings/");
	const unavailable = answer ? "Not installed on this machine" : "Pair this browser with Remote to open it";
	const modules = [...RAIL.map((fixed) => {
		const module = described.get(fixed.id);
		return {
			id: fixed.id,
			label: module?.rail.label ?? fixed.label,
			icon: module ? moduleIcon(module.rail.icon) : fixed.icon,
			href: module?.rail.path ?? (hosted(fixed.id) ? host.rail.path : `${place.origin}${fixed.path}`),
			needsYou: module?.needsYou,
			available: Boolean(module) || hosted(fixed.id) || HOST_SERVED.has(fixed.id)
		};
	}), ...[...described.values(), ...host && !described.has(host.extension) ? [host] : []].filter((module) => !RAIL.some((fixed) => fixed.id === module.extension)).sort((a, b) => a.rail.order - b.rail.order).map((module) => ({
		id: module.extension,
		label: module.rail.label,
		icon: moduleIcon(module.rail.icon),
		href: module.rail.path,
		needsYou: module.needsYou,
		available: true
	}))];
	const matched = active ?? modules.filter((module) => module.available && place.href.startsWith(module.href)).sort((a, b) => b.href.length - a.href.length)[0]?.id;
	const counts = [...described.values()].map((module) => module.needsYou);
	const total = answer && counts.every((count) => count !== void 0) ? counts.reduce((sum, count) => sum + count, 0) : void 0;
	return {
		main: [{
			id: "home",
			label: "Home",
			icon: "house",
			href: `${place.origin}/`,
			count: total ? shellCount(total) : void 0,
			current: path === "/" && !active
		}, ...modules.map((module) => ({
			id: module.id,
			label: module.label,
			icon: module.icon,
			href: module.href,
			count: module.needsYou ? shellCount(module.needsYou) : void 0,
			current: !settings && module.id === matched,
			...module.available ? {} : {
				disabled: true,
				note: unavailable
			}
		}))],
		bottom: [{
			id: "settings",
			label: "TMT settings",
			icon: "settings",
			href: `${place.origin}/settings/`,
			current: settings
		}]
	};
}
var RAIL_KEY = "tmt.shell.rail";
var SVG$1 = "http://www.w3.org/2000/svg";
function railCollapsed() {
	try {
		return localStorage.getItem(RAIL_KEY) === "collapsed";
	} catch {
		return false;
	}
}
function rememberRail(collapsed) {
	try {
		localStorage.setItem(RAIL_KEY, collapsed ? "collapsed" : "expanded");
	} catch {}
}
function icon(document, name) {
	const svg = document.createElementNS(SVG$1, "svg");
	svg.setAttribute("class", "tmt-ui-shell-icon");
	svg.setAttribute("viewBox", "0 0 24 24");
	svg.setAttribute("aria-hidden", "true");
	for (const [tag, attributes] of shellIconNodes[name]) {
		const node = document.createElementNS(SVG$1, tag);
		for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
		svg.append(node);
	}
	return svg;
}
function count(document, value) {
	const badge = document.createElement("span");
	badge.className = "tmt-ui-shell-count";
	if (value.needsYou) badge.dataset.kind = "needs-you";
	badge.textContent = value.needsYou ? `◆ ${value.text}` : value.text;
	return badge;
}
/**
* Render the rail and module menu into `root`, the `.tmt-ui-shell` element that holds
* `.tmt-ui-shell-content` with the host's `.tmt-ui-shell-bar` (its one header) and
* `.tmt-ui-shell-main`. On wide screens the TMT mark and the bar form one header row
* (TMT, then the host's `<module> / <page>`), over the rail, the module menu column and the main
* area; on phones the module menu becomes tabs under the title bar. `select` may take over a plain click
* on an item (an SPA route or Colab's search); it returns true when it handled the click.
*/
function mountShell(root, view, select) {
	const document = root.ownerDocument;
	root.classList.add("tmt-ui-shell");
	const open = document.createElement("button");
	open.type = "button";
	open.className = "tmt-ui-shell-open";
	open.setAttribute("aria-label", "Open navigation");
	open.setAttribute("aria-expanded", "false");
	open.append(icon(document, "menu"));
	const brand = document.createElement("span");
	brand.className = "tmt-ui-shell-brand";
	brand.innerHTML = brandMarkSvg({
		size: 24,
		id: "tmt-rail"
	});
	const word = document.createElement("span");
	word.className = "tmt-ui-shell-brand-word";
	word.textContent = "TMT";
	const dot = document.createElement("span");
	dot.className = "tmt-ui-shell-brand-separator";
	dot.setAttribute("aria-hidden", "true");
	dot.textContent = "·";
	brand.append(word, dot);
	const sheet = typeof ResizeObserver === "function" && "adoptedStyleSheets" in document ? new CSSStyleSheet() : void 0;
	if (sheet) document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
	const measure = sheet ? new ResizeObserver(() => sheet.replaceSync(`.tmt-ui-shell { --tmt-ui-shell-brand-width: ${brand.offsetWidth}px; }`)) : void 0;
	measure?.observe(brand);
	const rail = document.createElement("nav");
	rail.className = "tmt-ui-rail";
	rail.setAttribute("aria-label", "TMT");
	const menu = document.createElement("nav");
	menu.className = "tmt-ui-module-menu";
	root.prepend(open, brand, rail);
	root.querySelector(":scope > .tmt-ui-shell-content > .tmt-ui-shell-bar").after(menu);
	let collapsed = railCollapsed();
	const setDrawer = (shown) => {
		root.dataset.drawer = shown ? "open" : "closed";
		open.setAttribute("aria-expanded", String(shown));
	};
	const tip = document.createElement("div");
	tip.className = "tmt-ui-rail-tip";
	tip.setAttribute("role", "tooltip");
	tip.popover = "manual";
	root.append(tip);
	let release = () => {};
	const hideTip = () => {
		release();
		if (tip.matches(":popover-open")) tip.hidePopover();
	};
	const showTip = (anchor) => {
		const text = collapsed ? anchor.dataset.tip : anchor.dataset.note;
		if (!text || root.dataset.drawer === "open") return hideTip();
		release();
		release = claimIconActionTooltip(document, hideTip);
		tip.textContent = text;
		if (!tip.matches(":popover-open")) tip.showPopover();
		const box = anchor.getBoundingClientRect();
		tip.style.left = `${box.right}px`;
		tip.style.top = `${box.top + (box.height - tip.offsetHeight) / 2}px`;
	};
	const tipFor = (anchor, text, note) => {
		anchor.dataset.tip = text;
		if (note) anchor.dataset.note = text;
		anchor.addEventListener("pointerenter", () => showTip(anchor));
		anchor.addEventListener("focus", () => showTip(anchor));
		anchor.addEventListener("pointerleave", hideTip);
		anchor.addEventListener("blur", hideTip);
	};
	const paint = () => {
		root.dataset.rail = collapsed ? "collapsed" : "expanded";
		const label = collapsed ? "Expand sidebar" : "Collapse sidebar";
		toggle.setAttribute("aria-label", label);
		toggle.setAttribute("aria-expanded", String(!collapsed));
		toggle.dataset.tip = label;
		toggle.replaceChildren(icon(document, collapsed ? "panel-left-open" : "panel-left-close"));
	};
	const toggle = document.createElement("button");
	toggle.type = "button";
	toggle.className = "tmt-ui-rail-toggle";
	tipFor(toggle, "");
	toggle.onclick = () => {
		collapsed = !collapsed;
		rememberRail(collapsed);
		paint();
		hideTip();
	};
	const top = document.createElement("div");
	top.className = "tmt-ui-rail-top";
	top.append(toggle);
	open.onclick = () => setDrawer(root.dataset.drawer !== "open");
	const close = (event) => {
		if (event.key === "Escape" && root.dataset.drawer === "open") {
			setDrawer(false);
			open.focus();
		}
	};
	document.addEventListener("keydown", close);
	const outside = (event) => {
		const target = event.target;
		if (root.dataset.drawer === "open" && !rail.contains(target) && !open.contains(target)) setDrawer(false);
	};
	document.addEventListener("pointerdown", outside);
	const link = (className, item) => {
		const anchor = document.createElement("a");
		anchor.className = className;
		anchor.href = item.href;
		anchor.dataset.id = item.id;
		if (item.current) anchor.setAttribute("aria-current", "page");
		const disabled = "disabled" in item && item.disabled;
		if (disabled) anchor.setAttribute("aria-disabled", "true");
		anchor.onclick = (event) => {
			if (disabled) return event.preventDefault();
			if (event.button || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
			setDrawer(false);
			if (select?.(item.id, item.href)) event.preventDefault();
		};
		return anchor;
	};
	const railList = (items, extra) => {
		const list = document.createElement("ul");
		list.className = extra ? `tmt-ui-rail-items ${extra}` : "tmt-ui-rail-items";
		for (const item of items) {
			const anchor = link("tmt-ui-rail-item", item);
			tipFor(anchor, item.note ? `${item.label}: ${item.note}` : item.label, item.note);
			const label = document.createElement("span");
			label.className = "tmt-ui-rail-label";
			label.textContent = item.label;
			anchor.append(icon(document, item.icon), label);
			if (item.count) anchor.append(count(document, item.count));
			const row = document.createElement("li");
			row.append(anchor);
			list.append(row);
		}
		return list;
	};
	let shown = "";
	const render = (next) => {
		const key = JSON.stringify(next);
		if (key === shown) return;
		shown = key;
		hideTip();
		rail.replaceChildren(top, railList(next.rail.main), railList(next.rail.bottom, "tmt-ui-rail-bottom"));
		root.dataset.menu = next.menu ? "shown" : "none";
		if (!next.menu) {
			menu.replaceChildren();
			menu.removeAttribute("aria-label");
			return;
		}
		menu.setAttribute("aria-label", next.menu.title);
		const left = menu.scrollLeft;
		menu.replaceChildren();
		for (const group of next.menu.groups) {
			if (group.label !== void 0 || menu.childElementCount > 0) {
				const heading = document.createElement("div");
				heading.className = "tmt-ui-module-menu-group";
				heading.textContent = group.label ?? "";
				menu.append(heading);
			}
			const list = document.createElement("ul");
			list.className = "tmt-ui-module-menu-items";
			for (const item of group.items) {
				const anchor = link("tmt-ui-module-menu-item", item);
				if (item.note) anchor.title = item.note;
				const label = document.createElement("span");
				label.className = "tmt-ui-module-menu-label";
				label.textContent = item.label;
				if (!item.note) label.title = item.label;
				anchor.append(label);
				if (item.total !== void 0 || item.count) {
					const counts = document.createElement("span");
					counts.className = "tmt-ui-shell-counts";
					if (item.total !== void 0) counts.append(count(document, shellCount(item.total, "plain")));
					if (item.count) counts.append(count(document, item.count));
					anchor.append(counts);
				}
				const row = document.createElement("li");
				row.append(anchor);
				list.append(row);
			}
			menu.append(list);
		}
		menu.scrollLeft = left;
		const current = menu.querySelector("[aria-current=\"page\"]");
		if (current && menu.scrollWidth > menu.clientWidth) {
			const strip = menu.getBoundingClientRect();
			const box = current.getBoundingClientRect();
			if (box.left < strip.left) menu.scrollLeft -= strip.left - box.left;
			else if (box.right > strip.right) menu.scrollLeft += box.right - strip.right;
		}
	};
	paint();
	setDrawer(false);
	render(view);
	return {
		update: render,
		dispose() {
			document.removeEventListener("keydown", close);
			document.removeEventListener("pointerdown", outside);
			hideTip();
			measure?.disconnect();
			if (sheet) document.adoptedStyleSheets = document.adoptedStyleSheets.filter((item) => item !== sheet);
			open.remove();
			brand.remove();
			tip.remove();
			rail.remove();
			menu.remove();
		}
	};
}
//#endregion
//#region ../../../../design/browser-ui/src/shell-host.ts
function matchRoute(routes, pathname) {
	let best;
	let length = -1;
	for (const route of routes) for (const path of route.paths) if ((path === "/" ? pathname === "/" : pathname === path || path.endsWith("/") && pathname.startsWith(path)) && path.length > length) {
		best = route;
		length = path.length;
	}
	return best;
}
/** The rail as this app shows it: `route`'s module lit, every item linking to the app's own route
* for it rather than to a module's mount. */
function appRail(answer, routes, place, route) {
	const rail = shellRail(answer, place, route && route.rail !== "home" && route.rail !== "settings" ? {
		extension: route.rail,
		rail: {
			label: route.rail,
			icon: "box",
			path: `${place.origin}${route.paths.at(-1)}`,
			order: 0
		},
		menu: [],
		settings: []
	} : void 0);
	for (const item of [...rail.main, ...rail.bottom]) if (item.id !== "home") item.href = appHref(routes, item.href, item.id);
	return rail;
}
/** `href` as a link in this document resolves it: origin, path, query and fragment, as on a click. */
function linkTo(href) {
	const link = document.createElement("a");
	link.href = href;
	return link;
}
/**
* Where this app opens `href`: a path under a module's mount opens the same path under that
* module's route (`…/x/colab/pages/<id>` is `/colab/pages/<id>`), and a rail item `id` the app
* has a route for opens that route. Anything else is left as it is.
*/
function appHref(routes, href, id) {
	const url = linkTo(href);
	const mounted = underModuleMount(url.pathname);
	const base = routes.find((route) => route.rail === (mounted?.module ?? id))?.paths.at(-1);
	if (!base) return href;
	if (!mounted) return `${url.origin}${base}`;
	return `${url.origin}${base.replace(/\/?$/, "/")}${mounted.rest}${url.search}${url.hash}`;
}
/**
* Start the one app in `root`, a `.tmt-ui-shell` element holding `.tmt-ui-shell-content` with
* `.tmt-ui-shell-bar` and `.tmt-ui-shell-main`. A route the host does not own is a full load.
*/
function startShellHost(root, routes, options = {}) {
	const document = root.ownerDocument;
	const window = document.defaultView;
	const bar = root.querySelector(":scope > .tmt-ui-shell-content > .tmt-ui-shell-bar");
	const main = root.querySelector(":scope > .tmt-ui-shell-content > .tmt-ui-shell-main");
	const loading = document.createElement("div");
	loading.className = "tmt-ui-shell-loading";
	loading.setAttribute("role", "status");
	loading.setAttribute("aria-label", "Loading");
	loading.append(createBusyMark(document));
	main.after(loading);
	let answer = rememberedShellAnswer();
	let current;
	let menu;
	let select;
	let asked = false;
	let settled = false;
	const listeners = [];
	let generation = 0;
	const place = () => ({
		origin: window.location.origin,
		href: window.location.href.split(/[?#]/)[0]
	});
	/** The rail links to this app's routes, not to a module's own mount. */
	const view = () => ({
		rail: appRail(answer, routes, place(), current?.route),
		menu
	});
	const controls = mountShell(root, view(), (id, href) => {
		if (select?.(id, href)) return true;
		return go(href);
	});
	const paint = () => controls.update(view());
	const describe = (known) => {
		if (asked) return;
		asked = true;
		(known === void 0 ? shellAddress() : Promise.resolve(known)).then(async (address) => {
			const read = address === void 0 ? void 0 : await loadShellAnswer(address);
			if (read) {
				answer = read;
				paint();
			}
			settled = true;
			for (const listener of listeners.splice(0)) listener(read);
		});
	};
	const slot = (route, mine) => ({
		main,
		bar,
		basePath: route.paths.at(-1),
		answer: () => answer,
		setMenu(next, handler) {
			if (current?.generation !== mine) return;
			menu = next;
			select = handler;
			paint();
		},
		setTitle(title) {
			if (current?.generation === mine) document.title = `${title} · TMT`;
		},
		navigate: (href, options) => void go(href, options),
		describe,
		described(listener) {
			if (settled) listener(answer);
			else listeners.push(listener);
		}
	});
	/** The path a full load was last asked for, so a route the door serves but this app does not
	* know reloads once, then lands on Home instead of reloading forever. */
	const RELOADED = "tmt-shell-reloaded";
	const remember = (path) => {
		try {
			if (path === void 0) window.sessionStorage.removeItem(RELOADED);
			else window.sessionStorage.setItem(RELOADED, path);
			return true;
		} catch {
			return false;
		}
	};
	let reloaded = null;
	try {
		reloaded = window.sessionStorage.getItem(RELOADED);
	} catch {
		reloaded = null;
	}
	function elsewhere() {
		const path = window.location.pathname;
		if (reloaded !== path && remember(path)) return window.location.reload();
		remember(void 0);
		window.history.replaceState(null, "", "/");
		show();
	}
	async function show() {
		const route = matchRoute(routes, window.location.pathname);
		if (!route) return elsewhere();
		remember(void 0);
		if (current?.route === route) return paint();
		current?.release?.();
		const mine = ++generation;
		current = {
			route,
			generation: mine
		};
		menu = void 0;
		select = void 0;
		const label = [...view().rail.main, ...view().rail.bottom].find((item) => item.current)?.label;
		const crumb = document.createElement("header");
		crumb.className = "tmt-ui-header";
		crumb.dataset.shellCrumb = "";
		if (label && route.rail !== "home") {
			const word = document.createElement("span");
			word.className = "tmt-ui-wordmark";
			word.textContent = label;
			const brand = document.createElement("div");
			brand.className = "tmt-ui-brand";
			brand.append(word);
			crumb.append(brand);
		}
		bar.replaceChildren(crumb);
		main.replaceChildren();
		main.dataset.route = route.id;
		paint();
		if (!route.quiet) describe();
		try {
			const module = await route.load(answer, () => new Promise((resolve) => settled ? resolve(answer) : listeners.push(resolve)));
			if (current.generation !== mine) return;
			const release = await module.mount(slot(route, mine));
			if (current.generation !== mine) return release?.();
			current.release = release ?? void 0;
		} catch (error) {
			if (current.generation !== mine) return;
			main.replaceChildren();
			options.failed?.(main, route, error);
		}
	}
	/** Same-origin URL this app owns: change the history entry and show its route. */
	function go(href, { replace = false } = {}) {
		const url = linkTo(href);
		if (url.origin !== window.location.origin || !matchRoute(routes, url.pathname)) return false;
		const here = window.location;
		if (url.hash && url.pathname === here.pathname && url.search === here.search) {
			if (url.hash !== here.hash) window.location.hash = url.hash;
			return true;
		}
		if (url.href !== window.location.href) window.history[replace ? "replaceState" : "pushState"](null, "", url.href);
		if (matchRoute(routes, url.pathname) === current?.route) window.dispatchEvent(new PopStateEvent("popstate"));
		else show();
		return true;
	}
	const click = (event) => {
		if (event.defaultPrevented || event.button || event.metaKey || event.ctrlKey) return;
		if (event.shiftKey || event.altKey) return;
		const anchor = event.target?.closest?.("a[href]");
		if (!(anchor instanceof window.HTMLAnchorElement) || anchor.target || anchor.hasAttribute("download")) return;
		const url = anchor;
		const route = matchRoute(routes, url.pathname);
		if (url.origin !== window.location.origin || !route) return;
		if (route === current?.route) return;
		event.preventDefault();
		go(url.href);
	};
	const popstate = () => void show();
	document.addEventListener("click", click);
	window.addEventListener("popstate", popstate);
	show();
	return {
		navigate: (href, options) => void go(href, options),
		redescribe: () => new Promise((resolve) => {
			asked = false;
			settled = false;
			listeners.push(resolve);
			describe();
		}),
		route: (pathname) => matchRoute(routes, pathname),
		dispose() {
			loading.remove();
			document.removeEventListener("click", click);
			window.removeEventListener("popstate", popstate);
			current?.release?.();
			controls.dispose();
		}
	};
}
//#endregion
//#region ../../rust/tmt-remote/assets/home.html?raw
var home_default = "<!doctype html>\n<html lang=\"en\" data-tmt-page=\"home\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    <link rel=\"icon\" type=\"image/svg+xml\" href=\"data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI5NiIgaGVpZ2h0PSI5NiIgdmlld0JveD0iMCAwIDk2IDk2Ij48c3R5bGU+c3Zney0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWN5YW46IzFjZTBmZjstLXRtdC11aS1icmFuZC1tYXJrLWF1cm9yYS1ncmVlbjojNjJmMGI2Oy0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWdvbGQ6I2Y4ZjY4YTstLXRtdC11aS1icmFuZC1tYXJrLWZhcjojZmZmZmZmOy0tdG10LXVpLWJyYW5kLW1hcmstZ2xvdzojM2JlOGM2Oy0tdG10LXVpLWJyYW5kLW1hcmstcGFuZWwtdG9wOiMyZTM2NDI7LS10bXQtdWktYnJhbmQtbWFyay1wYW5lbC1ib3R0b206IzA1MDcwYTstLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLWVkZ2U6I2ZmZmZmZjMzOy0tdG10LXVpLWJyYW5kLW1hcmstcHJvbXB0OiNmZmZmZmY7LS10bXQtdWktYnJhbmQtbWFyay1tb25vLXN0YXJ0OiM2YTZhNmE7LS10bXQtdWktYnJhbmQtbWFyay1tb25vLWVuZDojYzhjOGM4Oy0tdG10LXVpLWJyYW5kLW1hcmstdGlsZTojMGUxMTE2Oy0tdG10LXVpLWJyYW5kLW1hcmstdGlsZS1lZGdlOiNmZmZmZmYyZX0udG10LWJyYW5kLW1hcmt7ZGlzcGxheTpibG9jaztmbGV4Om5vbmU7b3ZlcmZsb3c6dmlzaWJsZX0udG10LWJyYW5kLW1hcmstY3lhbntzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLWF1cm9yYS1jeWFuKX0udG10LWJyYW5kLW1hcmstZ3JlZW57c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1hdXJvcmEtZ3JlZW4pfS50bXQtYnJhbmQtbWFyay1nb2xke3N0b3AtY29sb3I6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWdvbGQpfS50bXQtYnJhbmQtbWFyay1mYXJ7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1mYXIpfS50bXQtYnJhbmQtbWFyay1nbG93e3N0b3AtY29sb3I6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstZ2xvdyl9LnRtdC1icmFuZC1tYXJrLXBhbmVsLXRvcHtzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLXRvcCl9LnRtdC1icmFuZC1tYXJrLXBhbmVsLWJvdHRvbXtzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLWJvdHRvbSl9LnRtdC1icmFuZC1tYXJrLW1vbm8tc3RhcnR7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1tb25vLXN0YXJ0KX0udG10LWJyYW5kLW1hcmstbW9uby1lbmR7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1tb25vLWVuZCl9LnRtdC1icmFuZC1tYXJrLXBhbmVse3N0cm9rZTp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1wYW5lbC1lZGdlKTtzdHJva2Utd2lkdGg6MS4yfS50bXQtYnJhbmQtbWFyay1yaW17ZmlsbDpub25lO3N0cm9rZS13aWR0aDoxLjE7c3Ryb2tlLWxpbmVjYXA6cm91bmR9LnRtdC1icmFuZC1tYXJrLXByb21wdHtmaWxsOm5vbmU7c3Ryb2tlOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXByb21wdCk7c3Ryb2tlLXdpZHRoOjMuOTtzdHJva2UtbGluZWNhcDpyb3VuZDtzdHJva2UtbGluZWpvaW46cm91bmR9LnRtdC1icmFuZC1tYXJrW2RhdGEtZm9ybT0nc21hbGwnXSAudG10LWJyYW5kLW1hcmstcHJvbXB0e3N0cm9rZS13aWR0aDo5fS50bXQtYnJhbmQtbWFya1tkYXRhLWZvcm09J3NtYWxsJ10gLnRtdC1icmFuZC1tYXJrLXBhbmVse3N0cm9rZS13aWR0aDoyLjV9LnRtdC1icmFuZC1tYXJrLXRpbGV7ZmlsbDp2YXIoLS10bXQtdWktYnJhbmQtbWFyay10aWxlKTtzdHJva2U6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstdGlsZS1lZGdlKX0udG10LWJyYW5kLW1hcmstcHJpbnR7ZGlzcGxheTpub25lfUBtZWRpYSBwcmludHsudG10LWJyYW5kLW1hcmstYmVhbXtkaXNwbGF5Om5vbmV9LnRtdC1icmFuZC1tYXJrLXByaW50e2Rpc3BsYXk6aW5saW5lfX08L3N0eWxlPjxyZWN0IGNsYXNzPSJ0bXQtYnJhbmQtbWFyay10aWxlIiB4PSIwLjUiIHk9IjAuNSIgd2lkdGg9Ijk1IiBoZWlnaHQ9Ijk1IiByeD0iMjIiPjwvcmVjdD48ZyBjbGFzcz0idG10LWJyYW5kLW1hcmsiIGRhdGEtZm9ybT0ic21hbGwiIHRyYW5zZm9ybT0idHJhbnNsYXRlKDE2IDE2KSBzY2FsZSgwLjY2NjcpIj48ZGVmcz48bGluZWFyR3JhZGllbnQgaWQ9InRhYi1hdXJvcmEiIHgxPSIwLjIiIHkxPSIwLjA1IiB4Mj0iMC44NSIgeTI9IjEiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1jeWFuIiBvZmZzZXQ9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstZ3JlZW4iIG9mZnNldD0iMC41Ij48L3N0b3A+PHN0b3AgY2xhc3M9InRtdC1icmFuZC1tYXJrLWdvbGQiIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItZmFyIiB4MT0iMCIgeTE9IjAiIHgyPSIxIiB5Mj0iMCI+PHN0b3AgY2xhc3M9InRtdC1icmFuZC1tYXJrLWZhciIgb2Zmc2V0PSIwLjQ1IiBzdG9wLW9wYWNpdHk9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstZmFyIiBvZmZzZXQ9IjEiIHN0b3Atb3BhY2l0eT0iMC4xNSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItcGFuZWwiIHgxPSIwIiB5MT0iMCIgeDI9IjAuNiIgeTI9IjEiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wYW5lbC10b3AiIG9mZnNldD0iMCI+PC9zdG9wPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wYW5lbC1ib3R0b20iIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItbW9ubyIgeDE9IjAiIHkxPSIwIiB4Mj0iMSIgeTI9IjAuMzUiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1tb25vLXN0YXJ0IiBvZmZzZXQ9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstbW9uby1lbmQiIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PC9kZWZzPjxnIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1iZWFtIj48cGF0aCBkPSJNNi4wMCAxNC4wMFE2IDE0IDYuMDAgMTQuMDBMNzcuMDcgMjIuODhRODYgMjQgODkuMDAgMjYuNTBMODkuMDAgMjYuNTBROTIgMjkgOTIuMDAgMzguMDBMOTIuMDAgNzIuMDBROTIgODEgODkuMDAgODMuNTBMODkuMDAgODMuNTBRODYgODYgNzcuMzcgODMuNDZMNTIuMDAgNzYuMDBRNTIgNzYgNTIuMDAgNzYuMDBMNi4wMCA3MC4wMFE2IDcwIDYuMDAgNzAuMDBaIiBmaWxsPSJ1cmwoI3RhYi1hdXJvcmEpIj48L3BhdGg+PHBhdGggZD0iTTYuMDAgMTQuMDBRNiAxNCA2LjAwIDE0LjAwTDc3LjA3IDIyLjg4UTg2IDI0IDg5LjAwIDI2LjUwTDg5LjAwIDI2LjUwUTkyIDI5IDkyLjAwIDM4LjAwTDkyLjAwIDcyLjAwUTkyIDgxIDg5LjAwIDgzLjUwTDg5LjAwIDgzLjUwUTg2IDg2IDc3LjM3IDgzLjQ2TDUyLjAwIDc2LjAwUTUyIDc2IDUyLjAwIDc2LjAwTDYuMDAgNzAuMDBRNiA3MCA2LjAwIDcwLjAwWiIgZmlsbD0idXJsKCN0YWItZmFyKSI+PC9wYXRoPjwvZz48cGF0aCBjbGFzcz0idG10LWJyYW5kLW1hcmstcHJpbnQiIGQ9Ik02LjAwIDE0LjAwUTYgMTQgNi4wMCAxNC4wMEw3Ny4wNyAyMi44OFE4NiAyNCA4OS4wMCAyNi41MEw4OS4wMCAyNi41MFE5MiAyOSA5Mi4wMCAzOC4wMEw5Mi4wMCA3Mi4wMFE5MiA4MSA4OS4wMCA4My41MEw4OS4wMCA4My41MFE4NiA4NiA3Ny4zNyA4My40Nkw1Mi4wMCA3Ni4wMFE1MiA3NiA1Mi4wMCA3Ni4wMEw2LjAwIDcwLjAwUTYgNzAgNi4wMCA3MC4wMFoiIGZpbGw9InVybCgjdGFiLW1vbm8pIj48L3BhdGg+PHBhdGggY2xhc3M9InRtdC1icmFuZC1tYXJrLXBhbmVsIiBkPSJNNS4wMCAyMS4wMFE1IDE0IDExLjg1IDE1LjQ2TDQ1LjE1IDIyLjU0UTUyIDI0IDUyLjAwIDMxLjAwTDUyLjAwIDY5LjAwUTUyIDc2IDQ1LjE1IDc3LjQ2TDExLjg1IDg0LjU0UTUgODYgNS4wMCA3OS4wMFoiIGZpbGw9InVybCgjdGFiLXBhbmVsKSI+PC9wYXRoPjxwYXRoIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wcm9tcHQiIGQ9Ik0xNSAzNyBMMjggNTAgTDE1IDYzIj48L3BhdGg+PC9nPjwvc3ZnPgo=\" />\n    <title>Home · TMT</title>\n    <link rel=\"stylesheet\" href=\"/sdk/pages.css\" />\n    <script type=\"module\" src=\"/sdk/shell.js\"><\/script>\n  </head>\n  <body>\n    <div class=\"tmt-ui-shell\">\n    <div class=\"tmt-ui-shell-content\">\n    <div class=\"tmt-ui-shell-bar\">\n    <header class=\"header tmt-ui-header\">\n      <h1 class=\"header-title tmt-ui-title\">Home</h1>\n    </header>\n    </div>\n    <div class=\"tmt-ui-shell-main\">\n    <main class=\"tmt-page tmt-home\">\n      <section class=\"tmt-ui-section\" aria-labelledby=\"home-needs-heading\">\n        <h2 id=\"home-needs-heading\" class=\"tmt-ui-section-heading\">◆ Needs you</h2>\n        <ul id=\"home-needs\" class=\"tmt-ui-list\"></ul>\n        <p id=\"home-needs-note\" class=\"home-note\" role=\"status\">Checking…</p>\n      </section>\n      <section id=\"home-recent-block\" class=\"tmt-ui-section\" aria-labelledby=\"home-recent-heading\" hidden>\n        <h2 id=\"home-recent-heading\" class=\"tmt-ui-section-heading\">Recent pages</h2>\n        <ul id=\"home-recent\" class=\"tmt-ui-list\"></ul>\n      </section>\n    </main>\n    </div>\n    </div>\n    </div>\n  </body>\n</html>\n";
//#endregion
//#region ../../rust/tmt-remote/assets/landing.html?raw
var landing_default = "<!doctype html>\n<html lang=\"en\" data-tmt-page=\"landing\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    <link rel=\"icon\" type=\"image/svg+xml\" href=\"data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI5NiIgaGVpZ2h0PSI5NiIgdmlld0JveD0iMCAwIDk2IDk2Ij48c3R5bGU+c3Zney0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWN5YW46IzFjZTBmZjstLXRtdC11aS1icmFuZC1tYXJrLWF1cm9yYS1ncmVlbjojNjJmMGI2Oy0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWdvbGQ6I2Y4ZjY4YTstLXRtdC11aS1icmFuZC1tYXJrLWZhcjojZmZmZmZmOy0tdG10LXVpLWJyYW5kLW1hcmstZ2xvdzojM2JlOGM2Oy0tdG10LXVpLWJyYW5kLW1hcmstcGFuZWwtdG9wOiMyZTM2NDI7LS10bXQtdWktYnJhbmQtbWFyay1wYW5lbC1ib3R0b206IzA1MDcwYTstLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLWVkZ2U6I2ZmZmZmZjMzOy0tdG10LXVpLWJyYW5kLW1hcmstcHJvbXB0OiNmZmZmZmY7LS10bXQtdWktYnJhbmQtbWFyay1tb25vLXN0YXJ0OiM2YTZhNmE7LS10bXQtdWktYnJhbmQtbWFyay1tb25vLWVuZDojYzhjOGM4Oy0tdG10LXVpLWJyYW5kLW1hcmstdGlsZTojMGUxMTE2Oy0tdG10LXVpLWJyYW5kLW1hcmstdGlsZS1lZGdlOiNmZmZmZmYyZX0udG10LWJyYW5kLW1hcmt7ZGlzcGxheTpibG9jaztmbGV4Om5vbmU7b3ZlcmZsb3c6dmlzaWJsZX0udG10LWJyYW5kLW1hcmstY3lhbntzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLWF1cm9yYS1jeWFuKX0udG10LWJyYW5kLW1hcmstZ3JlZW57c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1hdXJvcmEtZ3JlZW4pfS50bXQtYnJhbmQtbWFyay1nb2xke3N0b3AtY29sb3I6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWdvbGQpfS50bXQtYnJhbmQtbWFyay1mYXJ7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1mYXIpfS50bXQtYnJhbmQtbWFyay1nbG93e3N0b3AtY29sb3I6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstZ2xvdyl9LnRtdC1icmFuZC1tYXJrLXBhbmVsLXRvcHtzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLXRvcCl9LnRtdC1icmFuZC1tYXJrLXBhbmVsLWJvdHRvbXtzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLWJvdHRvbSl9LnRtdC1icmFuZC1tYXJrLW1vbm8tc3RhcnR7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1tb25vLXN0YXJ0KX0udG10LWJyYW5kLW1hcmstbW9uby1lbmR7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1tb25vLWVuZCl9LnRtdC1icmFuZC1tYXJrLXBhbmVse3N0cm9rZTp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1wYW5lbC1lZGdlKTtzdHJva2Utd2lkdGg6MS4yfS50bXQtYnJhbmQtbWFyay1yaW17ZmlsbDpub25lO3N0cm9rZS13aWR0aDoxLjE7c3Ryb2tlLWxpbmVjYXA6cm91bmR9LnRtdC1icmFuZC1tYXJrLXByb21wdHtmaWxsOm5vbmU7c3Ryb2tlOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXByb21wdCk7c3Ryb2tlLXdpZHRoOjMuOTtzdHJva2UtbGluZWNhcDpyb3VuZDtzdHJva2UtbGluZWpvaW46cm91bmR9LnRtdC1icmFuZC1tYXJrW2RhdGEtZm9ybT0nc21hbGwnXSAudG10LWJyYW5kLW1hcmstcHJvbXB0e3N0cm9rZS13aWR0aDo5fS50bXQtYnJhbmQtbWFya1tkYXRhLWZvcm09J3NtYWxsJ10gLnRtdC1icmFuZC1tYXJrLXBhbmVse3N0cm9rZS13aWR0aDoyLjV9LnRtdC1icmFuZC1tYXJrLXRpbGV7ZmlsbDp2YXIoLS10bXQtdWktYnJhbmQtbWFyay10aWxlKTtzdHJva2U6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstdGlsZS1lZGdlKX0udG10LWJyYW5kLW1hcmstcHJpbnR7ZGlzcGxheTpub25lfUBtZWRpYSBwcmludHsudG10LWJyYW5kLW1hcmstYmVhbXtkaXNwbGF5Om5vbmV9LnRtdC1icmFuZC1tYXJrLXByaW50e2Rpc3BsYXk6aW5saW5lfX08L3N0eWxlPjxyZWN0IGNsYXNzPSJ0bXQtYnJhbmQtbWFyay10aWxlIiB4PSIwLjUiIHk9IjAuNSIgd2lkdGg9Ijk1IiBoZWlnaHQ9Ijk1IiByeD0iMjIiPjwvcmVjdD48ZyBjbGFzcz0idG10LWJyYW5kLW1hcmsiIGRhdGEtZm9ybT0ic21hbGwiIHRyYW5zZm9ybT0idHJhbnNsYXRlKDE2IDE2KSBzY2FsZSgwLjY2NjcpIj48ZGVmcz48bGluZWFyR3JhZGllbnQgaWQ9InRhYi1hdXJvcmEiIHgxPSIwLjIiIHkxPSIwLjA1IiB4Mj0iMC44NSIgeTI9IjEiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1jeWFuIiBvZmZzZXQ9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstZ3JlZW4iIG9mZnNldD0iMC41Ij48L3N0b3A+PHN0b3AgY2xhc3M9InRtdC1icmFuZC1tYXJrLWdvbGQiIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItZmFyIiB4MT0iMCIgeTE9IjAiIHgyPSIxIiB5Mj0iMCI+PHN0b3AgY2xhc3M9InRtdC1icmFuZC1tYXJrLWZhciIgb2Zmc2V0PSIwLjQ1IiBzdG9wLW9wYWNpdHk9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstZmFyIiBvZmZzZXQ9IjEiIHN0b3Atb3BhY2l0eT0iMC4xNSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItcGFuZWwiIHgxPSIwIiB5MT0iMCIgeDI9IjAuNiIgeTI9IjEiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wYW5lbC10b3AiIG9mZnNldD0iMCI+PC9zdG9wPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wYW5lbC1ib3R0b20iIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItbW9ubyIgeDE9IjAiIHkxPSIwIiB4Mj0iMSIgeTI9IjAuMzUiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1tb25vLXN0YXJ0IiBvZmZzZXQ9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstbW9uby1lbmQiIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PC9kZWZzPjxnIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1iZWFtIj48cGF0aCBkPSJNNi4wMCAxNC4wMFE2IDE0IDYuMDAgMTQuMDBMNzcuMDcgMjIuODhRODYgMjQgODkuMDAgMjYuNTBMODkuMDAgMjYuNTBROTIgMjkgOTIuMDAgMzguMDBMOTIuMDAgNzIuMDBROTIgODEgODkuMDAgODMuNTBMODkuMDAgODMuNTBRODYgODYgNzcuMzcgODMuNDZMNTIuMDAgNzYuMDBRNTIgNzYgNTIuMDAgNzYuMDBMNi4wMCA3MC4wMFE2IDcwIDYuMDAgNzAuMDBaIiBmaWxsPSJ1cmwoI3RhYi1hdXJvcmEpIj48L3BhdGg+PHBhdGggZD0iTTYuMDAgMTQuMDBRNiAxNCA2LjAwIDE0LjAwTDc3LjA3IDIyLjg4UTg2IDI0IDg5LjAwIDI2LjUwTDg5LjAwIDI2LjUwUTkyIDI5IDkyLjAwIDM4LjAwTDkyLjAwIDcyLjAwUTkyIDgxIDg5LjAwIDgzLjUwTDg5LjAwIDgzLjUwUTg2IDg2IDc3LjM3IDgzLjQ2TDUyLjAwIDc2LjAwUTUyIDc2IDUyLjAwIDc2LjAwTDYuMDAgNzAuMDBRNiA3MCA2LjAwIDcwLjAwWiIgZmlsbD0idXJsKCN0YWItZmFyKSI+PC9wYXRoPjwvZz48cGF0aCBjbGFzcz0idG10LWJyYW5kLW1hcmstcHJpbnQiIGQ9Ik02LjAwIDE0LjAwUTYgMTQgNi4wMCAxNC4wMEw3Ny4wNyAyMi44OFE4NiAyNCA4OS4wMCAyNi41MEw4OS4wMCAyNi41MFE5MiAyOSA5Mi4wMCAzOC4wMEw5Mi4wMCA3Mi4wMFE5MiA4MSA4OS4wMCA4My41MEw4OS4wMCA4My41MFE4NiA4NiA3Ny4zNyA4My40Nkw1Mi4wMCA3Ni4wMFE1MiA3NiA1Mi4wMCA3Ni4wMEw2LjAwIDcwLjAwUTYgNzAgNi4wMCA3MC4wMFoiIGZpbGw9InVybCgjdGFiLW1vbm8pIj48L3BhdGg+PHBhdGggY2xhc3M9InRtdC1icmFuZC1tYXJrLXBhbmVsIiBkPSJNNS4wMCAyMS4wMFE1IDE0IDExLjg1IDE1LjQ2TDQ1LjE1IDIyLjU0UTUyIDI0IDUyLjAwIDMxLjAwTDUyLjAwIDY5LjAwUTUyIDc2IDQ1LjE1IDc3LjQ2TDExLjg1IDg0LjU0UTUgODYgNS4wMCA3OS4wMFoiIGZpbGw9InVybCgjdGFiLXBhbmVsKSI+PC9wYXRoPjxwYXRoIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wcm9tcHQiIGQ9Ik0xNSAzNyBMMjggNTAgTDE1IDYzIj48L3BhdGg+PC9nPjwvc3ZnPgo=\" />\n    <title>Remote connection · TMT</title>\n    <link rel=\"stylesheet\" href=\"/sdk/pages.css\" />\n    <script type=\"module\" src=\"/sdk/landing.js\"><\/script>\n  </head>\n  <body>\n    <div class=\"tmt-ui-shell\">\n    <div class=\"tmt-ui-shell-content\">\n    <div class=\"tmt-ui-shell-bar\">\n    <header class=\"header tmt-ui-header\">\n      <span class=\"header-brand tmt-ui-brand\">\n        <!-- Static-host copy of design/browser-ui/generated/brand-mark-static.svg. -->\n        <svg class=\"header-mark tmt-brand-mark tmt-ui-mark\" data-form=\"small\" viewBox=\"0 0 96 96\" aria-hidden=\"true\" width=\"24\" height=\"24\"><defs><linearGradient id=\"tmt-brand-aurora\" x1=\"0.2\" y1=\"0.05\" x2=\"0.85\" y2=\"1\"><stop class=\"tmt-brand-mark-cyan\" offset=\"0\"></stop><stop class=\"tmt-brand-mark-green\" offset=\"0.5\"></stop><stop class=\"tmt-brand-mark-gold\" offset=\"1\"></stop></linearGradient><linearGradient id=\"tmt-brand-far\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0\"><stop class=\"tmt-brand-mark-far\" offset=\"0.45\" stop-opacity=\"0\"></stop><stop class=\"tmt-brand-mark-far\" offset=\"1\" stop-opacity=\"0.15\"></stop></linearGradient><linearGradient id=\"tmt-brand-panel\" x1=\"0\" y1=\"0\" x2=\"0.6\" y2=\"1\"><stop class=\"tmt-brand-mark-panel-top\" offset=\"0\"></stop><stop class=\"tmt-brand-mark-panel-bottom\" offset=\"1\"></stop></linearGradient><linearGradient id=\"tmt-brand-mono\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0.35\"><stop class=\"tmt-brand-mark-mono-start\" offset=\"0\"></stop><stop class=\"tmt-brand-mark-mono-end\" offset=\"1\"></stop></linearGradient></defs><g class=\"tmt-brand-mark-beam\"><path d=\"M6.00 14.00Q6 14 6.00 14.00L77.07 22.88Q86 24 89.00 26.50L89.00 26.50Q92 29 92.00 38.00L92.00 72.00Q92 81 89.00 83.50L89.00 83.50Q86 86 77.37 83.46L52.00 76.00Q52 76 52.00 76.00L6.00 70.00Q6 70 6.00 70.00Z\" fill=\"url(#tmt-brand-aurora)\"></path><path d=\"M6.00 14.00Q6 14 6.00 14.00L77.07 22.88Q86 24 89.00 26.50L89.00 26.50Q92 29 92.00 38.00L92.00 72.00Q92 81 89.00 83.50L89.00 83.50Q86 86 77.37 83.46L52.00 76.00Q52 76 52.00 76.00L6.00 70.00Q6 70 6.00 70.00Z\" fill=\"url(#tmt-brand-far)\"></path></g><path class=\"tmt-brand-mark-print\" d=\"M6.00 14.00Q6 14 6.00 14.00L77.07 22.88Q86 24 89.00 26.50L89.00 26.50Q92 29 92.00 38.00L92.00 72.00Q92 81 89.00 83.50L89.00 83.50Q86 86 77.37 83.46L52.00 76.00Q52 76 52.00 76.00L6.00 70.00Q6 70 6.00 70.00Z\" fill=\"url(#tmt-brand-mono)\"></path><path class=\"tmt-brand-mark-panel\" d=\"M5.00 21.00Q5 14 11.85 15.46L45.15 22.54Q52 24 52.00 31.00L52.00 69.00Q52 76 45.15 77.46L11.85 84.54Q5 86 5.00 79.00Z\" fill=\"url(#tmt-brand-panel)\"></path><path class=\"tmt-brand-mark-prompt\" d=\"M15 37 L28 50 L15 63\"></path></svg><span class=\"header-wordmark tmt-ui-wordmark\">Remote</span>\n      </span>\n      <span class=\"header-divider\" aria-hidden=\"true\">/</span>\n      <h1 class=\"header-title tmt-ui-title\">Remote connection</h1>\n    </header>\n    </div>\n    <div class=\"tmt-ui-shell-main\">\n    <main class=\"tmt-page\">\n      <section\n        id=\"notice\"\n        class=\"sheet tmt-ui-notice\"\n        data-tone=\"waiting\"\n        aria-labelledby=\"heading\"\n      >\n        <div class=\"sheet-body tmt-ui-notice-body\">\n          <div class=\"tmt-ui-notice-mark\">\n            <span id=\"mark\" class=\"state-mark\" aria-hidden=\"true\">◌</span\n            ><span id=\"state-label\">Checking</span>\n          </div>\n          <h2 id=\"heading\" class=\"tmt-ui-notice-heading\">Checking the connection…</h2>\n          <p id=\"status\" role=\"status\">Reading this browser's pairing.</p>\n          <p id=\"command-location\" class=\"entry-note\" hidden>\n            Run these commands on the machine that runs Remote.\n          </p>\n          <ol id=\"steps-missing\" class=\"entry-steps\" hidden>\n            <li>\n              On the machine running Remote, run\n              <span class=\"entry-command tmt-ui-command\"\n                ><code class=\"tmt-ui-command-text\">tmt remote pair</code>\n                <button\n                  id=\"copy-missing-pair\"\n                  aria-label=\"Copy tmt remote pair\"\n                  type=\"button\"\n                  class=\"tmt-ui-command-copy\"\n                >\n                  Copy</button\n                ><span\n                  id=\"copy-feedback-missing-pair\"\n                  class=\"tmt-ui-command-feedback\"\n                  role=\"status\"\n                ></span\n              ></span>\n            </li>\n            <li>Open the link it prints <strong>in this browser</strong>.</li>\n            <li>Name this device, then check that the four words match the terminal.</li>\n            <li>Confirm in the terminal. This page updates when you come back.</li>\n          </ol>\n          <ol id=\"steps-different\" class=\"entry-steps\" hidden>\n            <li>\n              To use this machine too, run this on it:\n              <span class=\"entry-command tmt-ui-command\"\n                ><code class=\"tmt-ui-command-text\">tmt remote pair</code>\n                <button\n                  id=\"copy-different-pair\"\n                  aria-label=\"Copy tmt remote pair\"\n                  type=\"button\"\n                  class=\"tmt-ui-command-copy\"\n                >\n                  Copy</button\n                ><span\n                  id=\"copy-feedback-different-pair\"\n                  class=\"tmt-ui-command-feedback\"\n                  role=\"status\"\n                ></span\n              ></span>\n            </li>\n          </ol>\n          <ol id=\"steps-refused\" class=\"entry-steps\" hidden>\n            <li>\n              Check this device with\n              <span class=\"entry-command tmt-ui-command\"\n                ><code class=\"tmt-ui-command-text\">tmt remote devices</code>\n                <button\n                  id=\"copy-refused-devices\"\n                  aria-label=\"Copy tmt remote devices\"\n                  type=\"button\"\n                  class=\"tmt-ui-command-copy\"\n                >\n                  Copy</button\n                ><span\n                  id=\"copy-feedback-refused-devices\"\n                  class=\"tmt-ui-command-feedback\"\n                  role=\"status\"\n                ></span\n              ></span>\n            </li>\n            <li>\n              If it's missing, pair again with\n              <span class=\"entry-command tmt-ui-command\"\n                ><code class=\"tmt-ui-command-text\">tmt remote pair</code>\n                <button\n                  id=\"copy-refused-pair\"\n                  aria-label=\"Copy tmt remote pair\"\n                  type=\"button\"\n                  class=\"tmt-ui-command-copy\"\n                >\n                  Copy</button\n                ><span\n                  id=\"copy-feedback-refused-pair\"\n                  class=\"tmt-ui-command-feedback\"\n                  role=\"status\"\n                ></span\n              ></span>\n            </li>\n          </ol>\n          <ol id=\"steps-unconfirmed\" class=\"entry-steps\" hidden>\n            <li>\n              Check Remote is running:\n              <span class=\"entry-command tmt-ui-command\"\n                ><code class=\"tmt-ui-command-text\">tmt remote status</code>\n                <button\n                  id=\"copy-unconfirmed-status\"\n                  aria-label=\"Copy tmt remote status\"\n                  type=\"button\"\n                  class=\"tmt-ui-command-copy\"\n                >\n                  Copy</button\n                ><span\n                  id=\"copy-feedback-unconfirmed-status\"\n                  class=\"tmt-ui-command-feedback\"\n                  role=\"status\"\n                ></span\n              ></span>\n            </li>\n          </ol>\n          <ol id=\"steps-unreadable\" class=\"entry-steps\" hidden>\n            <li>\n              Pair again with\n              <span class=\"entry-command tmt-ui-command\"\n                ><code class=\"tmt-ui-command-text\">tmt remote pair</code>\n                <button\n                  id=\"copy-unreadable-pair\"\n                  aria-label=\"Copy tmt remote pair\"\n                  type=\"button\"\n                  class=\"tmt-ui-command-copy\"\n                >\n                  Copy</button\n                ><span\n                  id=\"copy-feedback-unreadable-pair\"\n                  class=\"tmt-ui-command-feedback\"\n                  role=\"status\"\n                ></span\n              ></span>\n            </li>\n          </ol>\n          <p id=\"pairing-note\" class=\"entry-note\" hidden>Pair links are private and work once.</p>\n          <button id=\"check\" class=\"tmt-ui-action\" type=\"button\" aria-describedby=\"status\" disabled>\n            Check again\n          </button>\n          <details class=\"tmt-ui-disclosure\" data-divided>\n            <summary>Details</summary>\n            <dl class=\"entry-status\">\n              <dt>Machine ID</dt>\n              <dd id=\"machine-id\">Not available</dd>\n              <dt>Last checked</dt>\n              <dd id=\"checked-time\">Not checked</dd>\n              <dt>Address</dt>\n              <dd id=\"protocol-address\">Not available</dd>\n              <dt>Fingerprint</dt>\n              <dd id=\"trust-pin\">Not available</dd>\n            </dl>\n            <p class=\"entry-note\">\n              Pairing gives this browser its own access. It does not make this browser a\n              settings administrator.\n            </p>\n          </details>\n        </div>\n      </section>\n      <section id=\"remotes\" class=\"tmt-ui-section\" data-divided aria-labelledby=\"remote-heading\">\n        <h2 id=\"remote-heading\" class=\"tmt-ui-section-heading\" data-variant=\"title\">Remotes in this browser</h2>\n        <p class=\"entry-note\">Names and addresses are saved here. Each remote keeps its own pairing.</p>\n        <ul id=\"remote-list\" class=\"tmt-ui-list\"></ul>\n        <p id=\"remote-status\" role=\"status\"></p>\n        <button id=\"remote-add\" class=\"tmt-ui-action\" type=\"button\">Add remote</button>\n      </section>\n    </main>\n    </div>\n    </div>\n    </div>\n  </body>\n</html>\n";
//#endregion
//#region ../../rust/tmt-remote/assets/settings.html?raw
var settings_default = "<!doctype html>\n<html lang=\"en\" data-tmt-page=\"settings\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    <link\n      rel=\"icon\"\n      type=\"image/svg+xml\"\n      href=\"data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI5NiIgaGVpZ2h0PSI5NiIgdmlld0JveD0iMCAwIDk2IDk2Ij48c3R5bGU+c3Zney0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWN5YW46IzFjZTBmZjstLXRtdC11aS1icmFuZC1tYXJrLWF1cm9yYS1ncmVlbjojNjJmMGI2Oy0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWdvbGQ6I2Y4ZjY4YTstLXRtdC11aS1icmFuZC1tYXJrLWZhcjojZmZmZmZmOy0tdG10LXVpLWJyYW5kLW1hcmstZ2xvdzojM2JlOGM2Oy0tdG10LXVpLWJyYW5kLW1hcmstcGFuZWwtdG9wOiMyZTM2NDI7LS10bXQtdWktYnJhbmQtbWFyay1wYW5lbC1ib3R0b206IzA1MDcwYTstLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLWVkZ2U6I2ZmZmZmZjMzOy0tdG10LXVpLWJyYW5kLW1hcmstcHJvbXB0OiNmZmZmZmY7LS10bXQtdWktYnJhbmQtbWFyay1tb25vLXN0YXJ0OiM2YTZhNmE7LS10bXQtdWktYnJhbmQtbWFyay1tb25vLWVuZDojYzhjOGM4Oy0tdG10LXVpLWJyYW5kLW1hcmstdGlsZTojMGUxMTE2Oy0tdG10LXVpLWJyYW5kLW1hcmstdGlsZS1lZGdlOiNmZmZmZmYyZX0udG10LWJyYW5kLW1hcmt7ZGlzcGxheTpibG9jaztmbGV4Om5vbmU7b3ZlcmZsb3c6dmlzaWJsZX0udG10LWJyYW5kLW1hcmstY3lhbntzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLWF1cm9yYS1jeWFuKX0udG10LWJyYW5kLW1hcmstZ3JlZW57c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1hdXJvcmEtZ3JlZW4pfS50bXQtYnJhbmQtbWFyay1nb2xke3N0b3AtY29sb3I6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWdvbGQpfS50bXQtYnJhbmQtbWFyay1mYXJ7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1mYXIpfS50bXQtYnJhbmQtbWFyay1nbG93e3N0b3AtY29sb3I6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstZ2xvdyl9LnRtdC1icmFuZC1tYXJrLXBhbmVsLXRvcHtzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLXRvcCl9LnRtdC1icmFuZC1tYXJrLXBhbmVsLWJvdHRvbXtzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLWJvdHRvbSl9LnRtdC1icmFuZC1tYXJrLW1vbm8tc3RhcnR7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1tb25vLXN0YXJ0KX0udG10LWJyYW5kLW1hcmstbW9uby1lbmR7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1tb25vLWVuZCl9LnRtdC1icmFuZC1tYXJrLXBhbmVse3N0cm9rZTp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1wYW5lbC1lZGdlKTtzdHJva2Utd2lkdGg6MS4yfS50bXQtYnJhbmQtbWFyay1yaW17ZmlsbDpub25lO3N0cm9rZS13aWR0aDoxLjE7c3Ryb2tlLWxpbmVjYXA6cm91bmR9LnRtdC1icmFuZC1tYXJrLXByb21wdHtmaWxsOm5vbmU7c3Ryb2tlOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXByb21wdCk7c3Ryb2tlLXdpZHRoOjMuOTtzdHJva2UtbGluZWNhcDpyb3VuZDtzdHJva2UtbGluZWpvaW46cm91bmR9LnRtdC1icmFuZC1tYXJrW2RhdGEtZm9ybT0nc21hbGwnXSAudG10LWJyYW5kLW1hcmstcHJvbXB0e3N0cm9rZS13aWR0aDo5fS50bXQtYnJhbmQtbWFya1tkYXRhLWZvcm09J3NtYWxsJ10gLnRtdC1icmFuZC1tYXJrLXBhbmVse3N0cm9rZS13aWR0aDoyLjV9LnRtdC1icmFuZC1tYXJrLXRpbGV7ZmlsbDp2YXIoLS10bXQtdWktYnJhbmQtbWFyay10aWxlKTtzdHJva2U6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstdGlsZS1lZGdlKX0udG10LWJyYW5kLW1hcmstcHJpbnR7ZGlzcGxheTpub25lfUBtZWRpYSBwcmludHsudG10LWJyYW5kLW1hcmstYmVhbXtkaXNwbGF5Om5vbmV9LnRtdC1icmFuZC1tYXJrLXByaW50e2Rpc3BsYXk6aW5saW5lfX08L3N0eWxlPjxyZWN0IGNsYXNzPSJ0bXQtYnJhbmQtbWFyay10aWxlIiB4PSIwLjUiIHk9IjAuNSIgd2lkdGg9Ijk1IiBoZWlnaHQ9Ijk1IiByeD0iMjIiPjwvcmVjdD48ZyBjbGFzcz0idG10LWJyYW5kLW1hcmsiIGRhdGEtZm9ybT0ic21hbGwiIHRyYW5zZm9ybT0idHJhbnNsYXRlKDE2IDE2KSBzY2FsZSgwLjY2NjcpIj48ZGVmcz48bGluZWFyR3JhZGllbnQgaWQ9InRhYi1hdXJvcmEiIHgxPSIwLjIiIHkxPSIwLjA1IiB4Mj0iMC44NSIgeTI9IjEiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1jeWFuIiBvZmZzZXQ9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstZ3JlZW4iIG9mZnNldD0iMC41Ij48L3N0b3A+PHN0b3AgY2xhc3M9InRtdC1icmFuZC1tYXJrLWdvbGQiIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItZmFyIiB4MT0iMCIgeTE9IjAiIHgyPSIxIiB5Mj0iMCI+PHN0b3AgY2xhc3M9InRtdC1icmFuZC1tYXJrLWZhciIgb2Zmc2V0PSIwLjQ1IiBzdG9wLW9wYWNpdHk9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstZmFyIiBvZmZzZXQ9IjEiIHN0b3Atb3BhY2l0eT0iMC4xNSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItcGFuZWwiIHgxPSIwIiB5MT0iMCIgeDI9IjAuNiIgeTI9IjEiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wYW5lbC10b3AiIG9mZnNldD0iMCI+PC9zdG9wPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wYW5lbC1ib3R0b20iIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItbW9ubyIgeDE9IjAiIHkxPSIwIiB4Mj0iMSIgeTI9IjAuMzUiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1tb25vLXN0YXJ0IiBvZmZzZXQ9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstbW9uby1lbmQiIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PC9kZWZzPjxnIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1iZWFtIj48cGF0aCBkPSJNNi4wMCAxNC4wMFE2IDE0IDYuMDAgMTQuMDBMNzcuMDcgMjIuODhRODYgMjQgODkuMDAgMjYuNTBMODkuMDAgMjYuNTBROTIgMjkgOTIuMDAgMzguMDBMOTIuMDAgNzIuMDBROTIgODEgODkuMDAgODMuNTBMODkuMDAgODMuNTBRODYgODYgNzcuMzcgODMuNDZMNTIuMDAgNzYuMDBRNTIgNzYgNTIuMDAgNzYuMDBMNi4wMCA3MC4wMFE2IDcwIDYuMDAgNzAuMDBaIiBmaWxsPSJ1cmwoI3RhYi1hdXJvcmEpIj48L3BhdGg+PHBhdGggZD0iTTYuMDAgMTQuMDBRNiAxNCA2LjAwIDE0LjAwTDc3LjA3IDIyLjg4UTg2IDI0IDg5LjAwIDI2LjUwTDg5LjAwIDI2LjUwUTkyIDI5IDkyLjAwIDM4LjAwTDkyLjAwIDcyLjAwUTkyIDgxIDg5LjAwIDgzLjUwTDg5LjAwIDgzLjUwUTg2IDg2IDc3LjM3IDgzLjQ2TDUyLjAwIDc2LjAwUTUyIDc2IDUyLjAwIDc2LjAwTDYuMDAgNzAuMDBRNiA3MCA2LjAwIDcwLjAwWiIgZmlsbD0idXJsKCN0YWItZmFyKSI+PC9wYXRoPjwvZz48cGF0aCBjbGFzcz0idG10LWJyYW5kLW1hcmstcHJpbnQiIGQ9Ik02LjAwIDE0LjAwUTYgMTQgNi4wMCAxNC4wMEw3Ny4wNyAyMi44OFE4NiAyNCA4OS4wMCAyNi41MEw4OS4wMCAyNi41MFE5MiAyOSA5Mi4wMCAzOC4wMEw5Mi4wMCA3Mi4wMFE5MiA4MSA4OS4wMCA4My41MEw4OS4wMCA4My41MFE4NiA4NiA3Ny4zNyA4My40Nkw1Mi4wMCA3Ni4wMFE1MiA3NiA1Mi4wMCA3Ni4wMEw2LjAwIDcwLjAwUTYgNzAgNi4wMCA3MC4wMFoiIGZpbGw9InVybCgjdGFiLW1vbm8pIj48L3BhdGg+PHBhdGggY2xhc3M9InRtdC1icmFuZC1tYXJrLXBhbmVsIiBkPSJNNS4wMCAyMS4wMFE1IDE0IDExLjg1IDE1LjQ2TDQ1LjE1IDIyLjU0UTUyIDI0IDUyLjAwIDMxLjAwTDUyLjAwIDY5LjAwUTUyIDc2IDQ1LjE1IDc3LjQ2TDExLjg1IDg0LjU0UTUgODYgNS4wMCA3OS4wMFoiIGZpbGw9InVybCgjdGFiLXBhbmVsKSI+PC9wYXRoPjxwYXRoIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wcm9tcHQiIGQ9Ik0xNSAzNyBMMjggNTAgTDE1IDYzIj48L3BhdGg+PC9nPjwvc3ZnPgo=\"\n    />\n    <title>Remote settings · TMT</title>\n    <link rel=\"stylesheet\" href=\"/sdk/pages.css\" />\n    <link rel=\"stylesheet\" href=\"/sdk/settings.css\" />\n    <script type=\"module\" src=\"/sdk/settings-v1.js\"><\/script>\n  </head>\n  <body>\n    <div class=\"tmt-ui-shell\">\n    <div class=\"tmt-ui-shell-content\">\n    <div class=\"tmt-ui-shell-bar\">\n    <header class=\"header tmt-ui-header\">\n      <span class=\"header-brand tmt-ui-brand\"\n        ><!-- Static-host copy of design/browser-ui/generated/brand-mark-static.svg. -->\n        <svg class=\"header-mark tmt-brand-mark tmt-ui-mark\" data-form=\"small\" viewBox=\"0 0 96 96\" aria-hidden=\"true\" width=\"24\" height=\"24\"><defs><linearGradient id=\"tmt-brand-aurora\" x1=\"0.2\" y1=\"0.05\" x2=\"0.85\" y2=\"1\"><stop class=\"tmt-brand-mark-cyan\" offset=\"0\"></stop><stop class=\"tmt-brand-mark-green\" offset=\"0.5\"></stop><stop class=\"tmt-brand-mark-gold\" offset=\"1\"></stop></linearGradient><linearGradient id=\"tmt-brand-far\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0\"><stop class=\"tmt-brand-mark-far\" offset=\"0.45\" stop-opacity=\"0\"></stop><stop class=\"tmt-brand-mark-far\" offset=\"1\" stop-opacity=\"0.15\"></stop></linearGradient><linearGradient id=\"tmt-brand-panel\" x1=\"0\" y1=\"0\" x2=\"0.6\" y2=\"1\"><stop class=\"tmt-brand-mark-panel-top\" offset=\"0\"></stop><stop class=\"tmt-brand-mark-panel-bottom\" offset=\"1\"></stop></linearGradient><linearGradient id=\"tmt-brand-mono\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0.35\"><stop class=\"tmt-brand-mark-mono-start\" offset=\"0\"></stop><stop class=\"tmt-brand-mark-mono-end\" offset=\"1\"></stop></linearGradient></defs><g class=\"tmt-brand-mark-beam\"><path d=\"M6.00 14.00Q6 14 6.00 14.00L77.07 22.88Q86 24 89.00 26.50L89.00 26.50Q92 29 92.00 38.00L92.00 72.00Q92 81 89.00 83.50L89.00 83.50Q86 86 77.37 83.46L52.00 76.00Q52 76 52.00 76.00L6.00 70.00Q6 70 6.00 70.00Z\" fill=\"url(#tmt-brand-aurora)\"></path><path d=\"M6.00 14.00Q6 14 6.00 14.00L77.07 22.88Q86 24 89.00 26.50L89.00 26.50Q92 29 92.00 38.00L92.00 72.00Q92 81 89.00 83.50L89.00 83.50Q86 86 77.37 83.46L52.00 76.00Q52 76 52.00 76.00L6.00 70.00Q6 70 6.00 70.00Z\" fill=\"url(#tmt-brand-far)\"></path></g><path class=\"tmt-brand-mark-print\" d=\"M6.00 14.00Q6 14 6.00 14.00L77.07 22.88Q86 24 89.00 26.50L89.00 26.50Q92 29 92.00 38.00L92.00 72.00Q92 81 89.00 83.50L89.00 83.50Q86 86 77.37 83.46L52.00 76.00Q52 76 52.00 76.00L6.00 70.00Q6 70 6.00 70.00Z\" fill=\"url(#tmt-brand-mono)\"></path><path class=\"tmt-brand-mark-panel\" d=\"M5.00 21.00Q5 14 11.85 15.46L45.15 22.54Q52 24 52.00 31.00L52.00 69.00Q52 76 45.15 77.46L11.85 84.54Q5 86 5.00 79.00Z\" fill=\"url(#tmt-brand-panel)\"></path><path class=\"tmt-brand-mark-prompt\" d=\"M15 37 L28 50 L15 63\"></path></svg><span class=\"header-wordmark tmt-ui-wordmark\">Remote</span></span\n      >\n      <span class=\"header-divider\" aria-hidden=\"true\">/</span>\n      <h1 class=\"header-title tmt-ui-title\">Remote settings</h1>\n    </header>\n    </div>\n    <div class=\"tmt-ui-shell-main\">\n    <main class=\"tmt-page settings-page\">\n      <h2 class=\"settings-title\">Remote settings</h2>\n      <p class=\"settings-intro\">\n        Control browser opening, sessions and paired devices for this Remote.\n      </p>\n      <section\n        id=\"access-notice\"\n        class=\"tmt-ui-notice\"\n        data-tone=\"waiting\"\n        aria-labelledby=\"access\"\n      >\n        <p class=\"eyebrow tmt-ui-notice-eyebrow\">Settings access</p>\n        <div class=\"tmt-ui-notice-mark\">\n          <span aria-hidden=\"true\">◆</span>\n          <p id=\"access\">Checking current access…</p>\n          <p\n            id=\"access-announcement\"\n            class=\"tmt-ui-visually-hidden\"\n            role=\"status\"\n            aria-live=\"polite\"\n            aria-atomic=\"true\"\n          ></p>\n        </div>\n        <p id=\"read-only\">\n          Read-only in this browser. To remove devices here, run this on the machine:\n        </p>\n        <div id=\"designation-command\" class=\"tmt-ui-command\" hidden>\n          <code id=\"designation-code\" class=\"tmt-ui-code\"></code\n          ><button\n            id=\"copy-designation\"\n            class=\"tmt-ui-command-copy\"\n            type=\"button\"\n            aria-label=\"Copy command\"\n          >\n            Copy\n          </button>\n        </div>\n        <p id=\"designation-copy-feedback\" role=\"status\"></p>\n        <p class=\"settings-cli\">\n          <code class=\"tmt-ui-code\">tmt remote settings</code> ·\n          <code class=\"tmt-ui-code\">tmt remote devices</code>\n        </p>\n        <p id=\"warning\" class=\"tmt-ui-field-error\" role=\"alert\" hidden></p>\n        <p id=\"controls-reason\">Checking current access before changes are available.</p>\n        <button\n          class=\"tmt-ui-action\"\n          aria-describedby=\"controls-reason\"\n          id=\"refresh\"\n          type=\"button\"\n          disabled\n        >\n          <span class=\"tmt-ui-action-label\">Refresh values</span>\n        </button>\n      </section>\n      <div class=\"change-feedback\" data-feedback=\"shared\">\n        <p id=\"outcome\" data-outcome-slot role=\"status\" aria-live=\"polite\" aria-atomic=\"true\"></p>\n        <p data-original hidden></p>\n        <button class=\"tmt-ui-action\" data-recover type=\"button\" hidden>\n          <span class=\"tmt-ui-action-label\">Check the saved value</span>\n        </button>\n      </div>\n      <section class=\"tmt-ui-section\" data-divided aria-labelledby=\"opening-title\">\n        <div class=\"settings-section-body\">\n          <div class=\"settings-summary\">\n            <h2 id=\"opening-title\" class=\"tmt-ui-section-heading\" data-variant=\"title\">\n              Browser opening\n            </h2>\n            <p id=\"opening-value\">Loading…</p>\n          </div>\n          <form class=\"settings-fields\" id=\"opening-form\" aria-describedby=\"read-only\">\n            <label id=\"opening-label\" class=\"tmt-ui-field-label\" for=\"opening\"\n              >Open a browser when pairing</label\n            >\n            <button\n              id=\"opening\"\n              class=\"tmt-ui-menu-trigger\" data-variant=\"select\"\n              type=\"button\"\n              aria-haspopup=\"listbox\"\n              aria-controls=\"opening-options\"\n              aria-expanded=\"false\"\n              aria-labelledby=\"opening-label opening-choice-value\"\n              value=\"on\"\n              disabled\n            >\n              <span id=\"opening-choice-value\" data-choice-value>Loading…</span\n              >\n            </button>\n            <div\n              id=\"opening-options\"\n              class=\"tmt-ui-menu-surface\"\n              popover=\"auto\"\n              role=\"listbox\"\n              aria-labelledby=\"opening-label\"\n            >\n              <button\n                class=\"tmt-ui-menu-item\"\n                type=\"button\"\n                role=\"option\"\n                data-value=\"on\"\n                aria-label=\"On\"\n                aria-selected=\"false\"\n                tabindex=\"-1\"\n                disabled\n              >\n                On\n              </button>\n              <button\n                class=\"tmt-ui-menu-item\"\n                type=\"button\"\n                role=\"option\"\n                data-value=\"off\"\n                aria-label=\"Off\"\n                aria-selected=\"false\"\n                tabindex=\"-1\"\n                disabled\n              >\n                Off\n              </button>\n            </div>\n            <div class=\"settings-actions\">\n              <button\n                class=\"tmt-ui-action\"\n                aria-describedby=\"controls-reason\"\n                type=\"submit\"\n                disabled\n              >\n                <span class=\"tmt-ui-action-label\">Save browser opening</span>\n              </button>\n\n              <p id=\"opening-unchanged\" class=\"tmt-ui-field-description\" hidden>\n                No changes to save.\n              </p>\n              <div class=\"change-feedback\" data-feedback=\"open\">\n                <p data-outcome-slot role=\"status\" aria-live=\"polite\" aria-atomic=\"true\"></p>\n                <p data-original hidden></p>\n                <button class=\"tmt-ui-action\" data-recover type=\"button\" hidden>\n                  <span class=\"tmt-ui-action-label\">Check the saved value</span>\n                </button>\n              </div>\n            </div>\n          </form>\n        </div>\n      </section>\n      <section class=\"tmt-ui-section\" data-divided aria-labelledby=\"limit-title\">\n        <div class=\"settings-section-body\">\n          <div class=\"settings-summary\">\n            <h2 id=\"limit-title\" class=\"tmt-ui-section-heading\" data-variant=\"title\">\n              Session limit per device\n            </h2>\n            <p id=\"limit-value\">Loading…</p>\n            <p id=\"limit-help\">\n              No limit by default. A limit can stop another tab from connecting; it does not remove\n              the browser's pairing.\n            </p>\n          </div>\n          <form class=\"settings-fields\" id=\"limit-form\" aria-describedby=\"limit-help read-only\">\n            <label id=\"limit-mode-label\" class=\"tmt-ui-field-label\" for=\"limit-mode\"\n              >Session limit</label\n            >\n            <button\n              id=\"limit-mode\"\n              class=\"tmt-ui-menu-trigger\" data-variant=\"select\"\n              type=\"button\"\n              aria-haspopup=\"listbox\"\n              aria-controls=\"limit-mode-options\"\n              aria-expanded=\"false\"\n              aria-labelledby=\"limit-mode-label limit-mode-choice-value\"\n              value=\"default\"\n              disabled\n            >\n              <span id=\"limit-mode-choice-value\" data-choice-value>Loading…</span\n              >\n            </button>\n            <div\n              id=\"limit-mode-options\"\n              class=\"tmt-ui-menu-surface\"\n              popover=\"auto\"\n              role=\"listbox\"\n              aria-labelledby=\"limit-mode-label\"\n            >\n              <button\n                class=\"tmt-ui-menu-item\"\n                type=\"button\"\n                role=\"option\"\n                data-value=\"default\"\n                aria-label=\"Default (off, not configured)\"\n                aria-selected=\"false\"\n                tabindex=\"-1\"\n                disabled\n              >\n                Default (off, not configured)\n              </button>\n              <button\n                class=\"tmt-ui-menu-item\"\n                type=\"button\"\n                role=\"option\"\n                data-value=\"off\"\n                aria-label=\"Off (unlimited)\"\n                aria-selected=\"false\"\n                tabindex=\"-1\"\n                disabled\n              >\n                Off (unlimited)\n              </button>\n              <button\n                class=\"tmt-ui-menu-item\"\n                type=\"button\"\n                role=\"option\"\n                data-value=\"custom\"\n                aria-label=\"Custom\"\n                aria-selected=\"false\"\n                tabindex=\"-1\"\n                disabled\n              >\n                Custom\n              </button>\n            </div>\n            <div class=\"tmt-ui-field\">\n              <label id=\"limit-custom-label\" class=\"tmt-ui-field-label\" for=\"limit-custom\"\n                >Positive integer</label\n              >\n              <input\n                id=\"limit-custom\"\n                class=\"tmt-ui-field-control\"\n                aria-labelledby=\"limit-custom-label\"\n                type=\"text\"\n                inputmode=\"numeric\"\n                pattern=\"[1-9][0-9]*\"\n                autocomplete=\"off\"\n                disabled\n              />\n            </div>\n            <div class=\"settings-actions\">\n              <button\n                class=\"tmt-ui-action\"\n                aria-describedby=\"controls-reason\"\n                type=\"submit\"\n                disabled\n              >\n                <span class=\"tmt-ui-action-label\">Save session limit</span>\n              </button>\n\n              <p id=\"limit-unchanged\" class=\"tmt-ui-field-description\" hidden>\n                No changes to save.\n              </p>\n              <div class=\"change-feedback\" data-feedback=\"sessions-per-device\">\n                <p data-outcome-slot role=\"status\" aria-live=\"polite\" aria-atomic=\"true\"></p>\n                <p data-original hidden></p>\n                <button class=\"tmt-ui-action\" data-recover type=\"button\" hidden>\n                  <span class=\"tmt-ui-action-label\">Check the saved value</span>\n                </button>\n              </div>\n            </div>\n          </form>\n        </div>\n      </section>\n      <section class=\"tmt-ui-section\" data-divided aria-labelledby=\"firestore-title\">\n        <div class=\"settings-section-body\">\n          <h2 id=\"firestore-title\" class=\"tmt-ui-section-heading\" data-variant=\"title\">Firestore</h2>\n          <p>Recorded setup, not a live Firebase check.</p>\n          <p\n            id=\"firestore-announcement\"\n            class=\"tmt-ui-visually-hidden\"\n            role=\"status\"\n            aria-live=\"polite\"\n            aria-atomic=\"true\"\n          ></p>\n          <div id=\"firestore-content\">Checking recorded Firestore setup…</div>\n        </div>\n      </section>\n      <section class=\"tmt-ui-section\" data-divided aria-labelledby=\"devices-title\">\n        <div class=\"settings-section-body\">\n          <h2 id=\"devices-title\" class=\"tmt-ui-section-heading\" data-variant=\"title\">Paired devices</h2>\n          <p>“This device” marks the pairing this browser uses. It does not give access to settings.</p>\n          <button\n            id=\"remove-others\"\n            class=\"tmt-ui-action\"\n            data-variant=\"destructive\"\n            type=\"button\"\n            disabled\n            aria-expanded=\"false\"\n            aria-controls=\"remove-others-confirmation\"\n          >\n            <span class=\"tmt-ui-action-label\">Remove the others</span>\n          </button>\n          <div id=\"remove-others-confirmation\" class=\"tmt-ui-notice\" hidden>\n            <p id=\"remove-others-consequence\" class=\"tmt-ui-notice-body\" role=\"status\"></p>\n            <div class=\"tmt-ui-notice-actions\">\n              <button id=\"cancel-remove-others\" class=\"tmt-ui-action\" type=\"button\">\n                <span class=\"tmt-ui-action-label\">Cancel</span>\n              </button>\n            </div>\n          </div>\n          <div id=\"devices\"></div>\n          <button\n            class=\"tmt-ui-action\"\n            aria-describedby=\"controls-reason\"\n            id=\"first\"\n            type=\"button\"\n            hidden\n          >\n            <span class=\"tmt-ui-action-label\">First devices</span>\n          </button>\n          <button\n            class=\"tmt-ui-action\"\n            aria-describedby=\"controls-reason\"\n            id=\"more\"\n            type=\"button\"\n            hidden\n          >\n            <span class=\"tmt-ui-action-label\">Next devices</span>\n          </button>\n        </div>\n      </section>\n      <section id=\"remotes\" class=\"tmt-ui-section\" data-divided aria-labelledby=\"remote-heading\">\n        <h2 id=\"remote-heading\" class=\"tmt-ui-section-heading\" data-variant=\"title\">Remotes in this browser</h2>\n        <p class=\"entry-note\">\n          Names and addresses are saved here. Each remote keeps its own pairing.\n        </p>\n        <ul id=\"remote-list\" class=\"tmt-ui-list\"></ul>\n        <p id=\"remote-status\" role=\"status\"></p>\n        <button id=\"remote-add\" class=\"tmt-ui-action\" type=\"button\">Add remote</button>\n      </section>\n    </main>\n    </div>\n    </div>\n    </div>\n  </body>\n</html>\n";
//#endregion
//#region ../../rust/tmt-remote/assets/tmt-settings.html?raw
var tmt_settings_default = "<!doctype html>\n<html lang=\"en\" data-tmt-page=\"tmt-settings\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    <link rel=\"icon\" type=\"image/svg+xml\" href=\"data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI5NiIgaGVpZ2h0PSI5NiIgdmlld0JveD0iMCAwIDk2IDk2Ij48c3R5bGU+c3Zney0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWN5YW46IzFjZTBmZjstLXRtdC11aS1icmFuZC1tYXJrLWF1cm9yYS1ncmVlbjojNjJmMGI2Oy0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWdvbGQ6I2Y4ZjY4YTstLXRtdC11aS1icmFuZC1tYXJrLWZhcjojZmZmZmZmOy0tdG10LXVpLWJyYW5kLW1hcmstZ2xvdzojM2JlOGM2Oy0tdG10LXVpLWJyYW5kLW1hcmstcGFuZWwtdG9wOiMyZTM2NDI7LS10bXQtdWktYnJhbmQtbWFyay1wYW5lbC1ib3R0b206IzA1MDcwYTstLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLWVkZ2U6I2ZmZmZmZjMzOy0tdG10LXVpLWJyYW5kLW1hcmstcHJvbXB0OiNmZmZmZmY7LS10bXQtdWktYnJhbmQtbWFyay1tb25vLXN0YXJ0OiM2YTZhNmE7LS10bXQtdWktYnJhbmQtbWFyay1tb25vLWVuZDojYzhjOGM4Oy0tdG10LXVpLWJyYW5kLW1hcmstdGlsZTojMGUxMTE2Oy0tdG10LXVpLWJyYW5kLW1hcmstdGlsZS1lZGdlOiNmZmZmZmYyZX0udG10LWJyYW5kLW1hcmt7ZGlzcGxheTpibG9jaztmbGV4Om5vbmU7b3ZlcmZsb3c6dmlzaWJsZX0udG10LWJyYW5kLW1hcmstY3lhbntzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLWF1cm9yYS1jeWFuKX0udG10LWJyYW5kLW1hcmstZ3JlZW57c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1hdXJvcmEtZ3JlZW4pfS50bXQtYnJhbmQtbWFyay1nb2xke3N0b3AtY29sb3I6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWdvbGQpfS50bXQtYnJhbmQtbWFyay1mYXJ7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1mYXIpfS50bXQtYnJhbmQtbWFyay1nbG93e3N0b3AtY29sb3I6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstZ2xvdyl9LnRtdC1icmFuZC1tYXJrLXBhbmVsLXRvcHtzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLXRvcCl9LnRtdC1icmFuZC1tYXJrLXBhbmVsLWJvdHRvbXtzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLWJvdHRvbSl9LnRtdC1icmFuZC1tYXJrLW1vbm8tc3RhcnR7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1tb25vLXN0YXJ0KX0udG10LWJyYW5kLW1hcmstbW9uby1lbmR7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1tb25vLWVuZCl9LnRtdC1icmFuZC1tYXJrLXBhbmVse3N0cm9rZTp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1wYW5lbC1lZGdlKTtzdHJva2Utd2lkdGg6MS4yfS50bXQtYnJhbmQtbWFyay1yaW17ZmlsbDpub25lO3N0cm9rZS13aWR0aDoxLjE7c3Ryb2tlLWxpbmVjYXA6cm91bmR9LnRtdC1icmFuZC1tYXJrLXByb21wdHtmaWxsOm5vbmU7c3Ryb2tlOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXByb21wdCk7c3Ryb2tlLXdpZHRoOjMuOTtzdHJva2UtbGluZWNhcDpyb3VuZDtzdHJva2UtbGluZWpvaW46cm91bmR9LnRtdC1icmFuZC1tYXJrW2RhdGEtZm9ybT0nc21hbGwnXSAudG10LWJyYW5kLW1hcmstcHJvbXB0e3N0cm9rZS13aWR0aDo5fS50bXQtYnJhbmQtbWFya1tkYXRhLWZvcm09J3NtYWxsJ10gLnRtdC1icmFuZC1tYXJrLXBhbmVse3N0cm9rZS13aWR0aDoyLjV9LnRtdC1icmFuZC1tYXJrLXRpbGV7ZmlsbDp2YXIoLS10bXQtdWktYnJhbmQtbWFyay10aWxlKTtzdHJva2U6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstdGlsZS1lZGdlKX0udG10LWJyYW5kLW1hcmstcHJpbnR7ZGlzcGxheTpub25lfUBtZWRpYSBwcmludHsudG10LWJyYW5kLW1hcmstYmVhbXtkaXNwbGF5Om5vbmV9LnRtdC1icmFuZC1tYXJrLXByaW50e2Rpc3BsYXk6aW5saW5lfX08L3N0eWxlPjxyZWN0IGNsYXNzPSJ0bXQtYnJhbmQtbWFyay10aWxlIiB4PSIwLjUiIHk9IjAuNSIgd2lkdGg9Ijk1IiBoZWlnaHQ9Ijk1IiByeD0iMjIiPjwvcmVjdD48ZyBjbGFzcz0idG10LWJyYW5kLW1hcmsiIGRhdGEtZm9ybT0ic21hbGwiIHRyYW5zZm9ybT0idHJhbnNsYXRlKDE2IDE2KSBzY2FsZSgwLjY2NjcpIj48ZGVmcz48bGluZWFyR3JhZGllbnQgaWQ9InRhYi1hdXJvcmEiIHgxPSIwLjIiIHkxPSIwLjA1IiB4Mj0iMC44NSIgeTI9IjEiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1jeWFuIiBvZmZzZXQ9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstZ3JlZW4iIG9mZnNldD0iMC41Ij48L3N0b3A+PHN0b3AgY2xhc3M9InRtdC1icmFuZC1tYXJrLWdvbGQiIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItZmFyIiB4MT0iMCIgeTE9IjAiIHgyPSIxIiB5Mj0iMCI+PHN0b3AgY2xhc3M9InRtdC1icmFuZC1tYXJrLWZhciIgb2Zmc2V0PSIwLjQ1IiBzdG9wLW9wYWNpdHk9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstZmFyIiBvZmZzZXQ9IjEiIHN0b3Atb3BhY2l0eT0iMC4xNSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItcGFuZWwiIHgxPSIwIiB5MT0iMCIgeDI9IjAuNiIgeTI9IjEiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wYW5lbC10b3AiIG9mZnNldD0iMCI+PC9zdG9wPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wYW5lbC1ib3R0b20iIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItbW9ubyIgeDE9IjAiIHkxPSIwIiB4Mj0iMSIgeTI9IjAuMzUiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1tb25vLXN0YXJ0IiBvZmZzZXQ9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstbW9uby1lbmQiIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PC9kZWZzPjxnIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1iZWFtIj48cGF0aCBkPSJNNi4wMCAxNC4wMFE2IDE0IDYuMDAgMTQuMDBMNzcuMDcgMjIuODhRODYgMjQgODkuMDAgMjYuNTBMODkuMDAgMjYuNTBROTIgMjkgOTIuMDAgMzguMDBMOTIuMDAgNzIuMDBROTIgODEgODkuMDAgODMuNTBMODkuMDAgODMuNTBRODYgODYgNzcuMzcgODMuNDZMNTIuMDAgNzYuMDBRNTIgNzYgNTIuMDAgNzYuMDBMNi4wMCA3MC4wMFE2IDcwIDYuMDAgNzAuMDBaIiBmaWxsPSJ1cmwoI3RhYi1hdXJvcmEpIj48L3BhdGg+PHBhdGggZD0iTTYuMDAgMTQuMDBRNiAxNCA2LjAwIDE0LjAwTDc3LjA3IDIyLjg4UTg2IDI0IDg5LjAwIDI2LjUwTDg5LjAwIDI2LjUwUTkyIDI5IDkyLjAwIDM4LjAwTDkyLjAwIDcyLjAwUTkyIDgxIDg5LjAwIDgzLjUwTDg5LjAwIDgzLjUwUTg2IDg2IDc3LjM3IDgzLjQ2TDUyLjAwIDc2LjAwUTUyIDc2IDUyLjAwIDc2LjAwTDYuMDAgNzAuMDBRNiA3MCA2LjAwIDcwLjAwWiIgZmlsbD0idXJsKCN0YWItZmFyKSI+PC9wYXRoPjwvZz48cGF0aCBjbGFzcz0idG10LWJyYW5kLW1hcmstcHJpbnQiIGQ9Ik02LjAwIDE0LjAwUTYgMTQgNi4wMCAxNC4wMEw3Ny4wNyAyMi44OFE4NiAyNCA4OS4wMCAyNi41MEw4OS4wMCAyNi41MFE5MiAyOSA5Mi4wMCAzOC4wMEw5Mi4wMCA3Mi4wMFE5MiA4MSA4OS4wMCA4My41MEw4OS4wMCA4My41MFE4NiA4NiA3Ny4zNyA4My40Nkw1Mi4wMCA3Ni4wMFE1MiA3NiA1Mi4wMCA3Ni4wMEw2LjAwIDcwLjAwUTYgNzAgNi4wMCA3MC4wMFoiIGZpbGw9InVybCgjdGFiLW1vbm8pIj48L3BhdGg+PHBhdGggY2xhc3M9InRtdC1icmFuZC1tYXJrLXBhbmVsIiBkPSJNNS4wMCAyMS4wMFE1IDE0IDExLjg1IDE1LjQ2TDQ1LjE1IDIyLjU0UTUyIDI0IDUyLjAwIDMxLjAwTDUyLjAwIDY5LjAwUTUyIDc2IDQ1LjE1IDc3LjQ2TDExLjg1IDg0LjU0UTUgODYgNS4wMCA3OS4wMFoiIGZpbGw9InVybCgjdGFiLXBhbmVsKSI+PC9wYXRoPjxwYXRoIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wcm9tcHQiIGQ9Ik0xNSAzNyBMMjggNTAgTDE1IDYzIj48L3BhdGg+PC9nPjwvc3ZnPgo=\" />\n    <title>TMT settings · TMT</title>\n    <link rel=\"stylesheet\" href=\"/sdk/pages.css\" />\n    <script type=\"module\" src=\"/sdk/shell.js\"><\/script>\n  </head>\n  <body>\n    <div class=\"tmt-ui-shell\">\n    <div class=\"tmt-ui-shell-content\">\n    <div class=\"tmt-ui-shell-bar\">\n    <header class=\"header tmt-ui-header\">\n      <h1 class=\"header-title tmt-ui-title\">TMT settings</h1>\n    </header>\n    </div>\n    <div class=\"tmt-ui-shell-main\">\n    <main class=\"tmt-page tmt-settings\">\n      <p id=\"tmt-settings-note\" class=\"usage-note\" role=\"note\" hidden></p>\n      <section id=\"tmt-settings-section\" class=\"tmt-settings-section tmt-ui-section\" aria-live=\"polite\"></section>\n    </main>\n    </div>\n    </div>\n    </div>\n  </body>\n</html>\n";
//#endregion
//#region ../../rust/tmt-remote/assets/ops.html?raw
var ops_default = "<!doctype html>\n<html lang=\"en\" data-tmt-page=\"ops\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    <link rel=\"icon\" type=\"image/svg+xml\" href=\"data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI5NiIgaGVpZ2h0PSI5NiIgdmlld0JveD0iMCAwIDk2IDk2Ij48c3R5bGU+c3Zney0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWN5YW46IzFjZTBmZjstLXRtdC11aS1icmFuZC1tYXJrLWF1cm9yYS1ncmVlbjojNjJmMGI2Oy0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWdvbGQ6I2Y4ZjY4YTstLXRtdC11aS1icmFuZC1tYXJrLWZhcjojZmZmZmZmOy0tdG10LXVpLWJyYW5kLW1hcmstZ2xvdzojM2JlOGM2Oy0tdG10LXVpLWJyYW5kLW1hcmstcGFuZWwtdG9wOiMyZTM2NDI7LS10bXQtdWktYnJhbmQtbWFyay1wYW5lbC1ib3R0b206IzA1MDcwYTstLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLWVkZ2U6I2ZmZmZmZjMzOy0tdG10LXVpLWJyYW5kLW1hcmstcHJvbXB0OiNmZmZmZmY7LS10bXQtdWktYnJhbmQtbWFyay1tb25vLXN0YXJ0OiM2YTZhNmE7LS10bXQtdWktYnJhbmQtbWFyay1tb25vLWVuZDojYzhjOGM4Oy0tdG10LXVpLWJyYW5kLW1hcmstdGlsZTojMGUxMTE2Oy0tdG10LXVpLWJyYW5kLW1hcmstdGlsZS1lZGdlOiNmZmZmZmYyZX0udG10LWJyYW5kLW1hcmt7ZGlzcGxheTpibG9jaztmbGV4Om5vbmU7b3ZlcmZsb3c6dmlzaWJsZX0udG10LWJyYW5kLW1hcmstY3lhbntzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLWF1cm9yYS1jeWFuKX0udG10LWJyYW5kLW1hcmstZ3JlZW57c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1hdXJvcmEtZ3JlZW4pfS50bXQtYnJhbmQtbWFyay1nb2xke3N0b3AtY29sb3I6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWdvbGQpfS50bXQtYnJhbmQtbWFyay1mYXJ7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1mYXIpfS50bXQtYnJhbmQtbWFyay1nbG93e3N0b3AtY29sb3I6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstZ2xvdyl9LnRtdC1icmFuZC1tYXJrLXBhbmVsLXRvcHtzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLXRvcCl9LnRtdC1icmFuZC1tYXJrLXBhbmVsLWJvdHRvbXtzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLWJvdHRvbSl9LnRtdC1icmFuZC1tYXJrLW1vbm8tc3RhcnR7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1tb25vLXN0YXJ0KX0udG10LWJyYW5kLW1hcmstbW9uby1lbmR7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1tb25vLWVuZCl9LnRtdC1icmFuZC1tYXJrLXBhbmVse3N0cm9rZTp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1wYW5lbC1lZGdlKTtzdHJva2Utd2lkdGg6MS4yfS50bXQtYnJhbmQtbWFyay1yaW17ZmlsbDpub25lO3N0cm9rZS13aWR0aDoxLjE7c3Ryb2tlLWxpbmVjYXA6cm91bmR9LnRtdC1icmFuZC1tYXJrLXByb21wdHtmaWxsOm5vbmU7c3Ryb2tlOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXByb21wdCk7c3Ryb2tlLXdpZHRoOjMuOTtzdHJva2UtbGluZWNhcDpyb3VuZDtzdHJva2UtbGluZWpvaW46cm91bmR9LnRtdC1icmFuZC1tYXJrW2RhdGEtZm9ybT0nc21hbGwnXSAudG10LWJyYW5kLW1hcmstcHJvbXB0e3N0cm9rZS13aWR0aDo5fS50bXQtYnJhbmQtbWFya1tkYXRhLWZvcm09J3NtYWxsJ10gLnRtdC1icmFuZC1tYXJrLXBhbmVse3N0cm9rZS13aWR0aDoyLjV9LnRtdC1icmFuZC1tYXJrLXRpbGV7ZmlsbDp2YXIoLS10bXQtdWktYnJhbmQtbWFyay10aWxlKTtzdHJva2U6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstdGlsZS1lZGdlKX0udG10LWJyYW5kLW1hcmstcHJpbnR7ZGlzcGxheTpub25lfUBtZWRpYSBwcmludHsudG10LWJyYW5kLW1hcmstYmVhbXtkaXNwbGF5Om5vbmV9LnRtdC1icmFuZC1tYXJrLXByaW50e2Rpc3BsYXk6aW5saW5lfX08L3N0eWxlPjxyZWN0IGNsYXNzPSJ0bXQtYnJhbmQtbWFyay10aWxlIiB4PSIwLjUiIHk9IjAuNSIgd2lkdGg9Ijk1IiBoZWlnaHQ9Ijk1IiByeD0iMjIiPjwvcmVjdD48ZyBjbGFzcz0idG10LWJyYW5kLW1hcmsiIGRhdGEtZm9ybT0ic21hbGwiIHRyYW5zZm9ybT0idHJhbnNsYXRlKDE2IDE2KSBzY2FsZSgwLjY2NjcpIj48ZGVmcz48bGluZWFyR3JhZGllbnQgaWQ9InRhYi1hdXJvcmEiIHgxPSIwLjIiIHkxPSIwLjA1IiB4Mj0iMC44NSIgeTI9IjEiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1jeWFuIiBvZmZzZXQ9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstZ3JlZW4iIG9mZnNldD0iMC41Ij48L3N0b3A+PHN0b3AgY2xhc3M9InRtdC1icmFuZC1tYXJrLWdvbGQiIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItZmFyIiB4MT0iMCIgeTE9IjAiIHgyPSIxIiB5Mj0iMCI+PHN0b3AgY2xhc3M9InRtdC1icmFuZC1tYXJrLWZhciIgb2Zmc2V0PSIwLjQ1IiBzdG9wLW9wYWNpdHk9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstZmFyIiBvZmZzZXQ9IjEiIHN0b3Atb3BhY2l0eT0iMC4xNSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItcGFuZWwiIHgxPSIwIiB5MT0iMCIgeDI9IjAuNiIgeTI9IjEiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wYW5lbC10b3AiIG9mZnNldD0iMCI+PC9zdG9wPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wYW5lbC1ib3R0b20iIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItbW9ubyIgeDE9IjAiIHkxPSIwIiB4Mj0iMSIgeTI9IjAuMzUiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1tb25vLXN0YXJ0IiBvZmZzZXQ9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstbW9uby1lbmQiIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PC9kZWZzPjxnIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1iZWFtIj48cGF0aCBkPSJNNi4wMCAxNC4wMFE2IDE0IDYuMDAgMTQuMDBMNzcuMDcgMjIuODhRODYgMjQgODkuMDAgMjYuNTBMODkuMDAgMjYuNTBROTIgMjkgOTIuMDAgMzguMDBMOTIuMDAgNzIuMDBROTIgODEgODkuMDAgODMuNTBMODkuMDAgODMuNTBRODYgODYgNzcuMzcgODMuNDZMNTIuMDAgNzYuMDBRNTIgNzYgNTIuMDAgNzYuMDBMNi4wMCA3MC4wMFE2IDcwIDYuMDAgNzAuMDBaIiBmaWxsPSJ1cmwoI3RhYi1hdXJvcmEpIj48L3BhdGg+PHBhdGggZD0iTTYuMDAgMTQuMDBRNiAxNCA2LjAwIDE0LjAwTDc3LjA3IDIyLjg4UTg2IDI0IDg5LjAwIDI2LjUwTDg5LjAwIDI2LjUwUTkyIDI5IDkyLjAwIDM4LjAwTDkyLjAwIDcyLjAwUTkyIDgxIDg5LjAwIDgzLjUwTDg5LjAwIDgzLjUwUTg2IDg2IDc3LjM3IDgzLjQ2TDUyLjAwIDc2LjAwUTUyIDc2IDUyLjAwIDc2LjAwTDYuMDAgNzAuMDBRNiA3MCA2LjAwIDcwLjAwWiIgZmlsbD0idXJsKCN0YWItZmFyKSI+PC9wYXRoPjwvZz48cGF0aCBjbGFzcz0idG10LWJyYW5kLW1hcmstcHJpbnQiIGQ9Ik02LjAwIDE0LjAwUTYgMTQgNi4wMCAxNC4wMEw3Ny4wNyAyMi44OFE4NiAyNCA4OS4wMCAyNi41MEw4OS4wMCAyNi41MFE5MiAyOSA5Mi4wMCAzOC4wMEw5Mi4wMCA3Mi4wMFE5MiA4MSA4OS4wMCA4My41MEw4OS4wMCA4My41MFE4NiA4NiA3Ny4zNyA4My40Nkw1Mi4wMCA3Ni4wMFE1MiA3NiA1Mi4wMCA3Ni4wMEw2LjAwIDcwLjAwUTYgNzAgNi4wMCA3MC4wMFoiIGZpbGw9InVybCgjdGFiLW1vbm8pIj48L3BhdGg+PHBhdGggY2xhc3M9InRtdC1icmFuZC1tYXJrLXBhbmVsIiBkPSJNNS4wMCAyMS4wMFE1IDE0IDExLjg1IDE1LjQ2TDQ1LjE1IDIyLjU0UTUyIDI0IDUyLjAwIDMxLjAwTDUyLjAwIDY5LjAwUTUyIDc2IDQ1LjE1IDc3LjQ2TDExLjg1IDg0LjU0UTUgODYgNS4wMCA3OS4wMFoiIGZpbGw9InVybCgjdGFiLXBhbmVsKSI+PC9wYXRoPjxwYXRoIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wcm9tcHQiIGQ9Ik0xNSAzNyBMMjggNTAgTDE1IDYzIj48L3BhdGg+PC9nPjwvc3ZnPgo=\" />\n    <title>Ops · TMT</title>\n    <link rel=\"stylesheet\" href=\"/sdk/pages.css\" />\n    <script type=\"module\" src=\"/sdk/shell.js\"><\/script>\n  </head>\n  <body>\n    <div class=\"tmt-ui-shell\">\n    <div class=\"tmt-ui-shell-content\">\n    <div class=\"tmt-ui-shell-bar\">\n    <header class=\"header tmt-ui-header\">\n      <span class=\"header-brand tmt-ui-brand\">\n        <!-- Static-host copy of design/browser-ui/generated/brand-mark-static.svg. -->\n        <svg class=\"header-mark tmt-brand-mark tmt-ui-mark\" data-form=\"small\" viewBox=\"0 0 96 96\" aria-hidden=\"true\" width=\"24\" height=\"24\"><defs><linearGradient id=\"tmt-brand-aurora\" x1=\"0.2\" y1=\"0.05\" x2=\"0.85\" y2=\"1\"><stop class=\"tmt-brand-mark-cyan\" offset=\"0\"></stop><stop class=\"tmt-brand-mark-green\" offset=\"0.5\"></stop><stop class=\"tmt-brand-mark-gold\" offset=\"1\"></stop></linearGradient><linearGradient id=\"tmt-brand-far\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0\"><stop class=\"tmt-brand-mark-far\" offset=\"0.45\" stop-opacity=\"0\"></stop><stop class=\"tmt-brand-mark-far\" offset=\"1\" stop-opacity=\"0.15\"></stop></linearGradient><linearGradient id=\"tmt-brand-panel\" x1=\"0\" y1=\"0\" x2=\"0.6\" y2=\"1\"><stop class=\"tmt-brand-mark-panel-top\" offset=\"0\"></stop><stop class=\"tmt-brand-mark-panel-bottom\" offset=\"1\"></stop></linearGradient><linearGradient id=\"tmt-brand-mono\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0.35\"><stop class=\"tmt-brand-mark-mono-start\" offset=\"0\"></stop><stop class=\"tmt-brand-mark-mono-end\" offset=\"1\"></stop></linearGradient></defs><g class=\"tmt-brand-mark-beam\"><path d=\"M6.00 14.00Q6 14 6.00 14.00L77.07 22.88Q86 24 89.00 26.50L89.00 26.50Q92 29 92.00 38.00L92.00 72.00Q92 81 89.00 83.50L89.00 83.50Q86 86 77.37 83.46L52.00 76.00Q52 76 52.00 76.00L6.00 70.00Q6 70 6.00 70.00Z\" fill=\"url(#tmt-brand-aurora)\"></path><path d=\"M6.00 14.00Q6 14 6.00 14.00L77.07 22.88Q86 24 89.00 26.50L89.00 26.50Q92 29 92.00 38.00L92.00 72.00Q92 81 89.00 83.50L89.00 83.50Q86 86 77.37 83.46L52.00 76.00Q52 76 52.00 76.00L6.00 70.00Q6 70 6.00 70.00Z\" fill=\"url(#tmt-brand-far)\"></path></g><path class=\"tmt-brand-mark-print\" d=\"M6.00 14.00Q6 14 6.00 14.00L77.07 22.88Q86 24 89.00 26.50L89.00 26.50Q92 29 92.00 38.00L92.00 72.00Q92 81 89.00 83.50L89.00 83.50Q86 86 77.37 83.46L52.00 76.00Q52 76 52.00 76.00L6.00 70.00Q6 70 6.00 70.00Z\" fill=\"url(#tmt-brand-mono)\"></path><path class=\"tmt-brand-mark-panel\" d=\"M5.00 21.00Q5 14 11.85 15.46L45.15 22.54Q52 24 52.00 31.00L52.00 69.00Q52 76 45.15 77.46L11.85 84.54Q5 86 5.00 79.00Z\" fill=\"url(#tmt-brand-panel)\"></path><path class=\"tmt-brand-mark-prompt\" d=\"M15 37 L28 50 L15 63\"></path></svg><span class=\"header-wordmark tmt-ui-wordmark\">Ops</span>\n      </span>\n      <span class=\"header-divider\" aria-hidden=\"true\">/</span>\n      <h1 class=\"header-title tmt-ui-title\">Ops</h1>\n    </header>\n    </div>\n    <div class=\"tmt-ui-shell-main\">\n    <main class=\"tmt-page ops\">\n      <p id=\"ops-note\" class=\"usage-note\" role=\"note\" hidden></p>\n      <section id=\"ops-section\" class=\"ops-section tmt-ui-section\" aria-live=\"polite\"></section>\n    </main>\n    </div>\n    </div>\n    </div>\n  </body>\n</html>\n";
//#endregion
//#region ../../rust/tmt-remote/assets/digest.html?raw
var digest_default = "<!doctype html>\n<html lang=\"en\" data-tmt-page=\"digest\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    <link rel=\"icon\" type=\"image/svg+xml\" href=\"data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI5NiIgaGVpZ2h0PSI5NiIgdmlld0JveD0iMCAwIDk2IDk2Ij48c3R5bGU+c3Zney0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWN5YW46IzFjZTBmZjstLXRtdC11aS1icmFuZC1tYXJrLWF1cm9yYS1ncmVlbjojNjJmMGI2Oy0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWdvbGQ6I2Y4ZjY4YTstLXRtdC11aS1icmFuZC1tYXJrLWZhcjojZmZmZmZmOy0tdG10LXVpLWJyYW5kLW1hcmstZ2xvdzojM2JlOGM2Oy0tdG10LXVpLWJyYW5kLW1hcmstcGFuZWwtdG9wOiMyZTM2NDI7LS10bXQtdWktYnJhbmQtbWFyay1wYW5lbC1ib3R0b206IzA1MDcwYTstLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLWVkZ2U6I2ZmZmZmZjMzOy0tdG10LXVpLWJyYW5kLW1hcmstcHJvbXB0OiNmZmZmZmY7LS10bXQtdWktYnJhbmQtbWFyay1tb25vLXN0YXJ0OiM2YTZhNmE7LS10bXQtdWktYnJhbmQtbWFyay1tb25vLWVuZDojYzhjOGM4Oy0tdG10LXVpLWJyYW5kLW1hcmstdGlsZTojMGUxMTE2Oy0tdG10LXVpLWJyYW5kLW1hcmstdGlsZS1lZGdlOiNmZmZmZmYyZX0udG10LWJyYW5kLW1hcmt7ZGlzcGxheTpibG9jaztmbGV4Om5vbmU7b3ZlcmZsb3c6dmlzaWJsZX0udG10LWJyYW5kLW1hcmstY3lhbntzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLWF1cm9yYS1jeWFuKX0udG10LWJyYW5kLW1hcmstZ3JlZW57c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1hdXJvcmEtZ3JlZW4pfS50bXQtYnJhbmQtbWFyay1nb2xke3N0b3AtY29sb3I6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstYXVyb3JhLWdvbGQpfS50bXQtYnJhbmQtbWFyay1mYXJ7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1mYXIpfS50bXQtYnJhbmQtbWFyay1nbG93e3N0b3AtY29sb3I6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstZ2xvdyl9LnRtdC1icmFuZC1tYXJrLXBhbmVsLXRvcHtzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLXRvcCl9LnRtdC1icmFuZC1tYXJrLXBhbmVsLWJvdHRvbXtzdG9wLWNvbG9yOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXBhbmVsLWJvdHRvbSl9LnRtdC1icmFuZC1tYXJrLW1vbm8tc3RhcnR7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1tb25vLXN0YXJ0KX0udG10LWJyYW5kLW1hcmstbW9uby1lbmR7c3RvcC1jb2xvcjp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1tb25vLWVuZCl9LnRtdC1icmFuZC1tYXJrLXBhbmVse3N0cm9rZTp2YXIoLS10bXQtdWktYnJhbmQtbWFyay1wYW5lbC1lZGdlKTtzdHJva2Utd2lkdGg6MS4yfS50bXQtYnJhbmQtbWFyay1yaW17ZmlsbDpub25lO3N0cm9rZS13aWR0aDoxLjE7c3Ryb2tlLWxpbmVjYXA6cm91bmR9LnRtdC1icmFuZC1tYXJrLXByb21wdHtmaWxsOm5vbmU7c3Ryb2tlOnZhcigtLXRtdC11aS1icmFuZC1tYXJrLXByb21wdCk7c3Ryb2tlLXdpZHRoOjMuOTtzdHJva2UtbGluZWNhcDpyb3VuZDtzdHJva2UtbGluZWpvaW46cm91bmR9LnRtdC1icmFuZC1tYXJrW2RhdGEtZm9ybT0nc21hbGwnXSAudG10LWJyYW5kLW1hcmstcHJvbXB0e3N0cm9rZS13aWR0aDo5fS50bXQtYnJhbmQtbWFya1tkYXRhLWZvcm09J3NtYWxsJ10gLnRtdC1icmFuZC1tYXJrLXBhbmVse3N0cm9rZS13aWR0aDoyLjV9LnRtdC1icmFuZC1tYXJrLXRpbGV7ZmlsbDp2YXIoLS10bXQtdWktYnJhbmQtbWFyay10aWxlKTtzdHJva2U6dmFyKC0tdG10LXVpLWJyYW5kLW1hcmstdGlsZS1lZGdlKX0udG10LWJyYW5kLW1hcmstcHJpbnR7ZGlzcGxheTpub25lfUBtZWRpYSBwcmludHsudG10LWJyYW5kLW1hcmstYmVhbXtkaXNwbGF5Om5vbmV9LnRtdC1icmFuZC1tYXJrLXByaW50e2Rpc3BsYXk6aW5saW5lfX08L3N0eWxlPjxyZWN0IGNsYXNzPSJ0bXQtYnJhbmQtbWFyay10aWxlIiB4PSIwLjUiIHk9IjAuNSIgd2lkdGg9Ijk1IiBoZWlnaHQ9Ijk1IiByeD0iMjIiPjwvcmVjdD48ZyBjbGFzcz0idG10LWJyYW5kLW1hcmsiIGRhdGEtZm9ybT0ic21hbGwiIHRyYW5zZm9ybT0idHJhbnNsYXRlKDE2IDE2KSBzY2FsZSgwLjY2NjcpIj48ZGVmcz48bGluZWFyR3JhZGllbnQgaWQ9InRhYi1hdXJvcmEiIHgxPSIwLjIiIHkxPSIwLjA1IiB4Mj0iMC44NSIgeTI9IjEiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1jeWFuIiBvZmZzZXQ9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstZ3JlZW4iIG9mZnNldD0iMC41Ij48L3N0b3A+PHN0b3AgY2xhc3M9InRtdC1icmFuZC1tYXJrLWdvbGQiIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItZmFyIiB4MT0iMCIgeTE9IjAiIHgyPSIxIiB5Mj0iMCI+PHN0b3AgY2xhc3M9InRtdC1icmFuZC1tYXJrLWZhciIgb2Zmc2V0PSIwLjQ1IiBzdG9wLW9wYWNpdHk9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstZmFyIiBvZmZzZXQ9IjEiIHN0b3Atb3BhY2l0eT0iMC4xNSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItcGFuZWwiIHgxPSIwIiB5MT0iMCIgeDI9IjAuNiIgeTI9IjEiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wYW5lbC10b3AiIG9mZnNldD0iMCI+PC9zdG9wPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wYW5lbC1ib3R0b20iIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PGxpbmVhckdyYWRpZW50IGlkPSJ0YWItbW9ubyIgeDE9IjAiIHkxPSIwIiB4Mj0iMSIgeTI9IjAuMzUiPjxzdG9wIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1tb25vLXN0YXJ0IiBvZmZzZXQ9IjAiPjwvc3RvcD48c3RvcCBjbGFzcz0idG10LWJyYW5kLW1hcmstbW9uby1lbmQiIG9mZnNldD0iMSI+PC9zdG9wPjwvbGluZWFyR3JhZGllbnQ+PC9kZWZzPjxnIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1iZWFtIj48cGF0aCBkPSJNNi4wMCAxNC4wMFE2IDE0IDYuMDAgMTQuMDBMNzcuMDcgMjIuODhRODYgMjQgODkuMDAgMjYuNTBMODkuMDAgMjYuNTBROTIgMjkgOTIuMDAgMzguMDBMOTIuMDAgNzIuMDBROTIgODEgODkuMDAgODMuNTBMODkuMDAgODMuNTBRODYgODYgNzcuMzcgODMuNDZMNTIuMDAgNzYuMDBRNTIgNzYgNTIuMDAgNzYuMDBMNi4wMCA3MC4wMFE2IDcwIDYuMDAgNzAuMDBaIiBmaWxsPSJ1cmwoI3RhYi1hdXJvcmEpIj48L3BhdGg+PHBhdGggZD0iTTYuMDAgMTQuMDBRNiAxNCA2LjAwIDE0LjAwTDc3LjA3IDIyLjg4UTg2IDI0IDg5LjAwIDI2LjUwTDg5LjAwIDI2LjUwUTkyIDI5IDkyLjAwIDM4LjAwTDkyLjAwIDcyLjAwUTkyIDgxIDg5LjAwIDgzLjUwTDg5LjAwIDgzLjUwUTg2IDg2IDc3LjM3IDgzLjQ2TDUyLjAwIDc2LjAwUTUyIDc2IDUyLjAwIDc2LjAwTDYuMDAgNzAuMDBRNiA3MCA2LjAwIDcwLjAwWiIgZmlsbD0idXJsKCN0YWItZmFyKSI+PC9wYXRoPjwvZz48cGF0aCBjbGFzcz0idG10LWJyYW5kLW1hcmstcHJpbnQiIGQ9Ik02LjAwIDE0LjAwUTYgMTQgNi4wMCAxNC4wMEw3Ny4wNyAyMi44OFE4NiAyNCA4OS4wMCAyNi41MEw4OS4wMCAyNi41MFE5MiAyOSA5Mi4wMCAzOC4wMEw5Mi4wMCA3Mi4wMFE5MiA4MSA4OS4wMCA4My41MEw4OS4wMCA4My41MFE4NiA4NiA3Ny4zNyA4My40Nkw1Mi4wMCA3Ni4wMFE1MiA3NiA1Mi4wMCA3Ni4wMEw2LjAwIDcwLjAwUTYgNzAgNi4wMCA3MC4wMFoiIGZpbGw9InVybCgjdGFiLW1vbm8pIj48L3BhdGg+PHBhdGggY2xhc3M9InRtdC1icmFuZC1tYXJrLXBhbmVsIiBkPSJNNS4wMCAyMS4wMFE1IDE0IDExLjg1IDE1LjQ2TDQ1LjE1IDIyLjU0UTUyIDI0IDUyLjAwIDMxLjAwTDUyLjAwIDY5LjAwUTUyIDc2IDQ1LjE1IDc3LjQ2TDExLjg1IDg0LjU0UTUgODYgNS4wMCA3OS4wMFoiIGZpbGw9InVybCgjdGFiLXBhbmVsKSI+PC9wYXRoPjxwYXRoIGNsYXNzPSJ0bXQtYnJhbmQtbWFyay1wcm9tcHQiIGQ9Ik0xNSAzNyBMMjggNTAgTDE1IDYzIj48L3BhdGg+PC9nPjwvc3ZnPgo=\" />\n    <title>Digest · TMT</title>\n    <link rel=\"stylesheet\" href=\"/sdk/pages.css\" />\n    <script type=\"module\" src=\"/sdk/shell.js\"><\/script>\n  </head>\n  <body>\n    <div class=\"tmt-ui-shell\">\n    <div class=\"tmt-ui-shell-content\">\n    <div class=\"tmt-ui-shell-bar\">\n    <header class=\"header tmt-ui-header\">\n      <h1 class=\"header-title tmt-ui-title\">Digest</h1>\n    </header>\n    </div>\n    <div class=\"tmt-ui-shell-main\">\n    <main class=\"tmt-page usage digest\">\n      <div class=\"digest-head\">\n        <p class=\"digest-subtitle\">Your team's usage, at a glance.</p>\n        <div class=\"digest-controls\">\n          <span id=\"usage-badge\" class=\"digest-badge\" hidden></span>\n          <p id=\"usage-note\" class=\"usage-note\" role=\"note\" hidden></p>\n          <div id=\"usage-ranges\" class=\"digest-segments\" role=\"tablist\" aria-label=\"Range\"></div>\n        </div>\n      </div>\n      <div id=\"usage-panel\" role=\"tabpanel\" aria-label=\"Usage report\">\n        <div id=\"digest-sections\" class=\"digest-sections\" aria-live=\"polite\"></div>\n      </div>\n    </main>\n    </div>\n    </div>\n    </div>\n  </body>\n</html>\n";
//#endregion
//#region ../../../../design/browser-ui/src/chart-math.ts
/**
* Stack `layers` bottom to top in the given order. In share mode every column sums to 100; a
* column with no tokens at all stays empty rather than dividing by zero.
*/
function stackLayers(layers, options = {}) {
	const count = Math.max(0, ...layers.map((layer) => layer.values.length));
	const totals = Array.from({ length: count }, (_, at) => layers.reduce((sum, layer) => sum + (layer.values[at] ?? 0), 0));
	const floor = Array.from({ length: count }, () => 0);
	return {
		layers: layers.map((layer) => {
			const lower = [...floor];
			const upper = lower.map((base, at) => {
				const raw = layer.values[at] ?? 0;
				const value = options.share ? totals[at] ? raw / totals[at] * 100 : 0 : raw;
				floor[at] = base + value;
				return base + value;
			});
			return {
				key: layer.key,
				label: layer.label,
				lower,
				upper,
				missing: Array.from({ length: count }, (_, at) => layer.values[at] == null)
			};
		}),
		max: options.share ? 100 : Math.max(0, ...floor)
	};
}
/**
* Keep the `keep` largest layers by their sum and fold the rest into one `other` layer, so a
* chart never needs more series colours than it has. The order of kept layers is largest first.
*/
function topLayers(layers, keep, other = {
	key: "other",
	label: "Other"
}) {
	const sum = (layer) => layer.values.reduce((total, v) => total + (v ?? 0), 0);
	const ranked = [...layers].sort((a, b) => sum(b) - sum(a) || a.label.localeCompare(b.label));
	if (ranked.length <= keep) return ranked;
	const rest = ranked.slice(keep);
	const width = Math.max(...rest.map((layer) => layer.values.length));
	const values = Array.from({ length: width }, (_, at) => {
		const seen = rest.filter((layer) => layer.values[at] != null);
		return seen.length ? seen.reduce((total, layer) => total + (layer.values[at] ?? 0), 0) : null;
	});
	return [...ranked.slice(0, keep), {
		...other,
		values
	}];
}
/** The smallest 1, 2, 2.5, 5 × 10^k at or above `max`; 1 for an empty domain. */
function niceMax(max) {
	if (!(max > 0) || !Number.isFinite(max)) return 1;
	const base = 10 ** Math.floor(Math.log10(max));
	for (const step of [
		1,
		2,
		2.5,
		5,
		10
	]) if (step * base >= max - base * 1e-9) return step * base;
	return 10 * base;
}
/** `count + 1` evenly spaced ticks from 0 to `max`. */
function ticks(max, count = 4) {
	return Array.from({ length: count + 1 }, (_, at) => max * at / count);
}
/** 1.9B, 48.2M, 310K, 940: compact token and message counts. */
function compactCount(value) {
	const abs = Math.abs(value);
	if (abs >= 1e9) return `${(value / 1e9).toFixed(1)}B`;
	if (abs >= 1e6) return `${(value / 1e6).toFixed(1)}M`;
	if (abs >= 1e3) return `${Math.round(value / 1e3)}K`;
	return String(Math.round(value));
}
/**
* Intensity of `value` against the largest `max`: null for a missing observation, 0 for a
* known zero, otherwise 1..`steps` (LEVELS by default) on a square-root scale so small counts stay visible next to
* one busy pair.
*/
function level(value, max, steps = 5) {
	if (value == null) return null;
	if (value <= 0 || max <= 0) return 0;
	return Math.min(steps, Math.max(1, Math.ceil(Math.sqrt(value / max) * steps)));
}
/** SVG path data for a polyline through `points`. */
function linePath(points) {
	return points.map(([x, y], at) => `${at ? "L" : "M"}${round(x)} ${round(y)}`).join("");
}
/** SVG path data for the band between an upper and a lower edge over the same x positions. */
function areaPath(xs, upper, lower) {
	if (!xs.length) return "";
	return `${xs.map((x, at) => `${at ? "L" : "M"}${round(x)} ${round(upper[at])}`).join("")}${xs.map((x, at) => `L${round(x)} ${round(lower[at])}`).reverse().join("")}Z`;
}
function round(value) {
	return Math.round(value * 100) / 100;
}
//#endregion
//#region ../../../../design/browser-ui/src/chart-dom.ts
/**
* DOM helpers the leaf's chart painters share. No style attributes in markup: every look comes from a
* `tmt-ui-chart-*` class, so a chart works under a strict style policy.
*/
var SVG = "http://www.w3.org/2000/svg";
function svgEl(name, attrs = {}, className) {
	const node = document.createElementNS(SVG, name);
	for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, String(value));
	if (className) node.setAttribute("class", className);
	return node;
}
function htmlEl(name, className, text) {
	const node = document.createElement(name);
	if (className) node.className = className;
	if (text !== void 0) node.textContent = text;
	return node;
}
/**
* Call `paint(width)` once now and again whenever `host` is laid out at a clearly different width,
* so SVG text keeps its size at every viewport. Without ResizeObserver (tests, old engines) the
* first paint stays.
*/
function paintAtWidth(host, initial, paint) {
	let current = initial;
	paint(current);
	if (typeof ResizeObserver === "undefined") return;
	new ResizeObserver(() => {
		const width = Math.floor(host.getBoundingClientRect().width);
		if (width >= 120 && Math.abs(width - current) >= 8) {
			current = width;
			paint(width);
		}
	}).observe(host);
}
/** A table for assistive technology and copy-paste, in a visually hidden `tmt-ui-chart-data` box. */
function dataTable(caption, columns, rows) {
	const table = htmlEl("table");
	table.append(htmlEl("caption", void 0, caption));
	const head = htmlEl("tr");
	head.append(htmlEl("th", void 0, ""));
	for (const column of columns) {
		const cell = htmlEl("th", void 0, column);
		cell.scope = "col";
		head.append(cell);
	}
	const thead = htmlEl("thead");
	thead.append(head);
	table.append(thead);
	const body = htmlEl("tbody");
	for (const [label, ...values] of rows) {
		const line = htmlEl("tr");
		const header = htmlEl("th", void 0, label);
		header.scope = "row";
		line.append(header, ...values.map((value) => htmlEl("td", void 0, value)));
		body.append(line);
	}
	table.append(body);
	const box = htmlEl("div", "tmt-ui-chart-data");
	box.append(table);
	return box;
}
//#endregion
//#region ../../../../design/browser-ui/src/chart-matrix-model.ts
/** The matrix as plain data: every cell's text, intensity and spoken label. */
function matrixModel(options) {
	const people = options.participants;
	let max = 0;
	for (const from of people) for (const to of people) if (from.id !== to.id) max = Math.max(max, options.value(from.id, to.id) ?? 0);
	return {
		columns: [...people.map((person) => person.label), ...options.sum ? ["sum"] : []],
		rows: people.map((from) => {
			let sent = null;
			const cells = people.map((to) => {
				if (from.id === to.id) return {
					text: "",
					level: 0,
					self: true,
					label: `${from.label} to itself`
				};
				const count = options.value(from.id, to.id);
				if (count != null) sent = (sent ?? 0) + count;
				return {
					text: count == null ? "—" : count === 0 ? "·" : compactCount(count),
					level: level(count, max) ?? 0,
					self: false,
					label: `${from.label} to ${to.label}: ${count == null ? "unavailable" : count}`
				};
			});
			return {
				label: from.label,
				cells,
				sum: options.sum ? sent == null ? "—" : compactCount(sent) : void 0
			};
		})
	};
}
//#endregion
//#region ../../../../design/browser-ui/src/chart-matrix.ts
/**
* Who talks to whom: sender rows, recipient columns. A cell reads a number, `·` (observed none)
* or `—` (unavailable); the diagonal is blank. Intensity is a square-root step of the busiest
* pair, carried by class names that keep their text readable on both themes.
*/
function createMatrix(options) {
	const figure = htmlEl("figure", "tmt-ui-chart");
	figure.append(htmlEl("figcaption", "tmt-ui-chart-title", options.title));
	if (!options.participants.length) {
		figure.append(htmlEl("p", "tmt-ui-chart-empty", options.empty ?? "No agents observed."));
		return figure;
	}
	const model = matrixModel(options);
	const table = htmlEl("table", "tmt-ui-chart-matrix");
	const head = htmlEl("tr");
	const corner = htmlEl("th", "tmt-ui-chart-matrix-corner", "from\\to");
	corner.scope = "col";
	head.append(corner);
	for (const label of model.columns) {
		const column = htmlEl("th", "tmt-ui-chart-matrix-column");
		column.scope = "col";
		column.title = label;
		column.append(htmlEl("span", "tmt-ui-chart-matrix-name", label));
		head.append(column);
	}
	const thead = htmlEl("thead");
	thead.append(head);
	const body = htmlEl("tbody");
	for (const row of model.rows) {
		const line = htmlEl("tr");
		const name = htmlEl("th", "tmt-ui-chart-matrix-row");
		name.scope = "row";
		name.title = row.label;
		name.append(htmlEl("span", "tmt-ui-chart-matrix-name", row.label));
		line.append(name);
		for (const cell of row.cells) {
			const node = htmlEl("td", "tmt-ui-chart-cell", cell.text);
			node.setAttribute("aria-label", cell.label);
			if (cell.self) node.classList.add("tmt-ui-chart-cell-self");
			if (cell.level) node.classList.add(`tmt-ui-chart-level-${cell.level}`);
			line.append(node);
		}
		if (row.sum !== void 0) line.append(htmlEl("td", "tmt-ui-chart-cell tmt-ui-chart-cell-sum", row.sum));
		body.append(line);
	}
	table.append(thead, body);
	const scroll = htmlEl("div", "tmt-ui-chart-scroll");
	scroll.tabIndex = 0;
	scroll.setAttribute("role", "region");
	scroll.setAttribute("aria-label", options.title);
	scroll.append(table);
	figure.append(scroll);
	return figure;
}
var OTHER_STYLE = 7;
var styleOf = (layer, i, first = 0) => layer.key === "other" ? OTHER_STYLE : (first + i) % 7;
var MARGIN = {
	left: 46,
	right: 10,
	top: 12,
	bottom: 22
};
/** Everything a series chart paints, as plain data: the painter only maps it to elements. */
function seriesModel(options, width) {
	const columns = options.x.length;
	const share = options.kind === "share";
	const format = share ? (v) => `${Math.round(v)}%` : options.format ?? compactCount;
	const stack = options.kind === "stack" || share ? stackLayers(options.series, { share }) : void 0;
	const dataMax = stack ? stack.max : Math.max(0, ...options.series.flatMap((layer) => layer.values.map((v) => v ?? 0)), ...options.reference?.values.map((v) => v ?? 0) ?? []);
	const max = options.max ?? (share ? 100 : niceMax(dataMax));
	const height = options.height ?? 200;
	const inner = {
		left: MARGIN.left,
		right: Math.max(MARGIN.left + 1, width - MARGIN.right),
		top: MARGIN.top,
		bottom: height - MARGIN.bottom
	};
	const step = columns > 1 ? (inner.right - inner.left) / (columns - 1) : 0;
	const xAt = (i) => columns > 1 ? inner.left + i * step : (inner.left + inner.right) / 2;
	const yAt = (v) => inner.bottom - Math.min(v, max) / max * (inner.bottom - inner.top);
	const xs = options.x.map((_, i) => xAt(i));
	const marks = [];
	if (stack) stack.layers.forEach((layer, i) => marks.push({
		kind: "area",
		style: styleOf(layer, i, options.style),
		d: areaPath(xs, layer.upper.map(yAt), layer.lower.map(yAt))
	}));
	else if (options.kind === "bars") {
		const bar = Math.max(1, Math.min(24, (inner.right - inner.left) / columns - 2));
		options.series[0].values.forEach((v, i) => {
			if (v == null) return;
			const y = yAt(v);
			marks.push({
				kind: "bar",
				style: 0,
				rect: {
					x: xAt(i) - bar / 2,
					y,
					width: bar,
					height: Math.max(0, inner.bottom - y)
				}
			});
		});
	} else options.series.forEach((layer, i) => {
		let run = [];
		const flush = () => {
			if (run.length) marks.push({
				kind: "line",
				style: styleOf(layer, i, options.style),
				d: linePath(run)
			});
			run = [];
		};
		layer.values.forEach((v, at) => {
			if (v == null) flush();
			else run.push([xAt(at), yAt(v)]);
		});
		flush();
	});
	const run = [];
	options.reference?.values.forEach((v, at) => {
		if (v != null) run.push([xAt(at), yAt(v)]);
	});
	const totals = options.x.map((_, at) => options.series.reduce((sum, layer) => sum + (layer.values[at] ?? 0), 0));
	const cell = (layer, at) => {
		const v = layer.values[at];
		if (v == null) return "—";
		return share ? format(totals[at] ? v / totals[at] * 100 : 0) : format(v);
	};
	const edge = (at) => columns > 1 && at === 0 ? "start" : columns > 1 && at === columns - 1 ? "end" : "middle";
	return {
		width,
		height,
		grid: ticks(max, 4).map((tick) => ({
			y: yAt(tick),
			x1: inner.left,
			x2: inner.right,
			label: format(tick)
		})),
		xLabels: [.../* @__PURE__ */ new Set([
			0,
			Math.floor((columns - 1) / 2),
			columns - 1
		])].map((at) => ({
			x: xAt(at),
			text: options.x[at],
			anchor: edge(at)
		})),
		marks,
		reference: run.length > 1 ? linePath(run) : void 0,
		guides: (options.guides ?? []).map((guide) => ({
			x: xAt(guide.at),
			anchor: xAt(guide.at) > inner.right - 60 ? "end" : "start",
			top: inner.top,
			bottom: inner.bottom,
			label: guide.label
		})),
		xs,
		top: inner.top,
		bottom: inner.bottom,
		step,
		left: inner.left,
		values: (at) => options.series.map((layer) => cell(layer, at)),
		readout: (at) => `${options.x[at]}: ${options.series.map((layer) => `${layer.label} ${cell(layer, at)}`).join(", ")}`,
		legend: options.series.map((layer, i) => ({
			label: layer.label,
			style: styleOf(layer, i, options.style)
		})),
		tip: (at) => ({
			title: options.x[at] ?? "",
			rows: options.series.map((layer, i) => ({
				label: layer.label,
				value: cell(layer, at),
				style: styleOf(layer, i, options.style)
			}))
		}),
		table: {
			columns: [...options.x],
			rows: options.series.map((layer) => [layer.label, ...options.x.map((_, at) => cell(layer, at))])
		}
	};
}
//#endregion
//#region ../../../../design/browser-ui/src/chart-series.ts
/** A time-series chart: stacked areas, a 100 % share, columns or lines on one axis. */
function createSeriesChart(options) {
	const figure = htmlEl("figure", "tmt-ui-chart");
	figure.setAttribute("role", "group");
	figure.setAttribute("aria-label", options.title);
	figure.append(htmlEl("figcaption", "tmt-ui-chart-title", options.title));
	if (!options.series.some((layer) => layer.values.some((v) => v != null)) || !options.x.length) {
		figure.append(htmlEl("p", "tmt-ui-chart-empty", options.empty ?? "No observations yet."));
		return figure;
	}
	const plot = svgEl("svg", {}, "tmt-ui-chart-plot");
	plot.setAttribute("role", "img");
	plot.setAttribute("tabindex", "0");
	plot.setAttribute("aria-label", `${options.title}. Arrow keys read each position.`);
	const readout = htmlEl("p", "tmt-ui-chart-readout");
	readout.setAttribute("aria-live", "polite");
	const frame = htmlEl("div", "tmt-ui-chart-frame");
	const tip = htmlEl("div", "tmt-ui-chart-tip");
	tip.hidden = true;
	tip.setAttribute("aria-hidden", "true");
	const legend = htmlEl("ul", "tmt-ui-chart-legend");
	const first = seriesModel(options, 640);
	const valueNodes = first.legend.map(({ label, style }) => {
		const item = htmlEl("li", "tmt-ui-chart-legend-item");
		const swatch = htmlEl("span", `tmt-ui-chart-swatch tmt-ui-chart-s${style}`);
		swatch.setAttribute("aria-hidden", "true");
		item.append(swatch, htmlEl("span", "tmt-ui-chart-legend-label", label));
		const value = options.kind === "line" ? htmlEl("span", "tmt-ui-chart-legend-value") : void 0;
		if (value) item.append(value);
		legend.append(item);
		return value;
	});
	if (options.reference) {
		const item = htmlEl("li", "tmt-ui-chart-legend-item");
		const swatch = htmlEl("span", "tmt-ui-chart-swatch tmt-ui-chart-swatch-reference");
		swatch.setAttribute("aria-hidden", "true");
		item.append(swatch, htmlEl("span", "tmt-ui-chart-legend-label", options.reference.label));
		legend.append(item);
	}
	let model = first;
	let active = options.x.length - 1;
	/** The hover tip beside the column, flipped to the left half's other side near the right edge. */
	function showTip(at) {
		const data = model.tip(at);
		const title = htmlEl("div", "tmt-ui-chart-tip-title", data.title);
		const rows = data.rows.map((row) => {
			const line = htmlEl("div", "tmt-ui-chart-tip-row");
			const swatch = htmlEl("span", `tmt-ui-chart-swatch tmt-ui-chart-s${row.style}`);
			line.append(swatch, htmlEl("span", "tmt-ui-chart-tip-label", row.label), htmlEl("span", "tmt-ui-chart-legend-value", row.value));
			return line;
		});
		tip.replaceChildren(title, ...rows);
		tip.hidden = false;
		const x = model.xs[at] ?? 0;
		const flip = x > model.width / 2;
		tip.classList.toggle("tmt-ui-chart-tip-left", flip);
		tip.style.left = `${flip ? x - 12 : x + 12}px`;
	}
	function select(at) {
		active = Math.max(0, Math.min(options.x.length - 1, at));
		model.values(active).forEach((text, i) => {
			const node = valueNodes[i];
			if (node) node.textContent = text;
		});
		readout.textContent = model.readout(active);
		const cursor = plot.querySelector(".tmt-ui-chart-cursor");
		const x = String(model.xs[active] ?? 0);
		cursor?.setAttribute("x1", x);
		cursor?.setAttribute("x2", x);
		if (!tip.hidden) showTip(active);
	}
	function paint(width) {
		model = seriesModel(options, width);
		plot.replaceChildren();
		plot.setAttribute("width", String(model.width));
		plot.setAttribute("height", String(model.height));
		plot.setAttribute("viewBox", `0 0 ${model.width} ${model.height}`);
		const text = (x, y, label, anchor) => {
			const node = svgEl("text", {
				x,
				y
			}, "tmt-ui-chart-tick");
			if (anchor) node.setAttribute("text-anchor", anchor);
			node.textContent = label;
			plot.append(node);
		};
		for (const line of model.grid) {
			plot.append(svgEl("line", {
				x1: line.x1,
				x2: line.x2,
				y1: line.y,
				y2: line.y
			}, "tmt-ui-chart-grid"));
			text(line.x1 - 6, line.y + 4, line.label, "end");
		}
		for (const label of model.xLabels) text(label.x, model.height - 6, label.text, label.anchor);
		for (const mark of model.marks) if (mark.kind === "bar") plot.append(svgEl("rect", { ...mark.rect }, `tmt-ui-chart-bar tmt-ui-chart-s${mark.style}`));
		else plot.append(svgEl("path", { d: mark.d }, `tmt-ui-chart-${mark.kind} tmt-ui-chart-s${mark.style}`));
		if (model.reference) plot.append(svgEl("path", { d: model.reference }, "tmt-ui-chart-reference"));
		for (const guide of model.guides) {
			plot.append(svgEl("line", {
				x1: guide.x,
				x2: guide.x,
				y1: guide.top,
				y2: guide.bottom
			}, "tmt-ui-chart-guide"));
			text(guide.anchor === "end" ? guide.x - 4 : guide.x + 4, guide.top + 10, guide.label, guide.anchor);
		}
		plot.append(svgEl("line", {
			x1: 0,
			x2: 0,
			y1: model.top,
			y2: model.bottom
		}, "tmt-ui-chart-cursor"));
		select(active);
	}
	plot.addEventListener("keydown", (event) => {
		const to = event.key === "ArrowLeft" ? active - 1 : event.key === "ArrowRight" ? active + 1 : event.key === "Home" ? 0 : event.key === "End" ? options.x.length - 1 : void 0;
		if (to === void 0) return;
		event.preventDefault();
		select(to);
	});
	const pointer = (event) => {
		if (!model.step) return;
		select(Math.round((event.clientX - plot.getBoundingClientRect().left - model.left) / model.step));
	};
	const point = (event) => {
		pointer(event);
		showTip(active);
	};
	plot.addEventListener("pointermove", point);
	plot.addEventListener("pointerdown", point);
	plot.addEventListener("pointerleave", () => tip.hidden = true);
	plot.addEventListener("focus", () => showTip(active));
	plot.addEventListener("blur", () => tip.hidden = true);
	frame.append(plot, tip);
	figure.append(frame, readout, legend, dataTable(options.title, first.table.columns, first.table.rows));
	paintAtWidth(figure, 640, paint);
	return figure;
}
/**
* Each row scaled to its own busiest slot, so a quiet seat still shows its pattern. An unknown
* slot stays hollow and a known quiet one is the empty step: no data never reads as silence.
*/
function stripsModel(options) {
	const format = options.format ?? compactCount;
	return { rows: options.rows.map((row) => {
		const max = Math.max(0, ...row.slots.map((v) => v ?? 0));
		const active = row.slots.filter((v) => (v ?? 0) > 0).length;
		const known = row.slots.filter((v) => v != null).length;
		return {
			label: row.label,
			summary: `${row.label}: active in ${active} of ${known} observed slots`,
			slots: Array.from({ length: options.slotCount }, (_, at) => {
				const value = row.slots[at];
				const range = options.slotRange(at);
				const text = value == null ? "not observed" : `${format(value)} tokens`;
				return {
					level: level(value, max, 4),
					tip: {
						seat: row.label,
						range,
						value: text
					},
					label: `${row.label}, ${range}: ${text}`
				};
			})
		};
	}) };
}
//#endregion
//#region ../../../../design/browser-ui/src/chart-strips.ts
/** `Less ▢▢▢▢▢ More`: the swatches of the intensity steps, for a strip's caption or card header. */
function stripsLegend() {
	const legend = htmlEl("div", "tmt-ui-chart-strip-legend");
	legend.setAttribute("aria-hidden", "true");
	legend.append(htmlEl("span", void 0, "Less"));
	for (let step = 0; step <= 4; step += 1) legend.append(htmlEl("span", `tmt-ui-chart-slot tmt-ui-chart-slot-${step}`));
	legend.append(htmlEl("span", void 0, "More"));
	return legend;
}
/**
* Rows of fixed square slots, like a contribution graph: when each seat was active, scaled to
* that row's busiest slot. The row is as long as its slot count; a slot's tip (hover or focus)
* names the seat, the time range and the tokens.
*/
function createStrips(options) {
	const figure = htmlEl("figure", "tmt-ui-chart");
	figure.append(htmlEl("figcaption", "tmt-ui-chart-title", options.title));
	if (!options.rows.length) {
		figure.append(htmlEl("p", "tmt-ui-chart-empty", options.empty ?? "No agents observed."));
		return figure;
	}
	const frame = htmlEl("div", "tmt-ui-chart-frame");
	const list = htmlEl("ul", "tmt-ui-chart-strips");
	list.style.setProperty("--tmt-chart-strip-count", String(options.slotCount));
	const tip = htmlEl("div", "tmt-ui-chart-tip");
	tip.hidden = true;
	tip.setAttribute("aria-hidden", "true");
	const grid = [];
	for (const row of stripsModel(options).rows) {
		const item = htmlEl("li", "tmt-ui-chart-strip-row");
		item.setAttribute("aria-label", row.summary);
		const name = htmlEl("span", "tmt-ui-chart-strip-label", row.label);
		name.title = row.label;
		const strip = htmlEl("div", "tmt-ui-chart-strip");
		const cells = [];
		for (const slot of row.slots) {
			const node = htmlEl("span", "tmt-ui-chart-slot");
			node.classList.add(slot.level === null ? "tmt-ui-chart-slot-none" : `tmt-ui-chart-slot-${slot.level}`);
			node.setAttribute("role", "img");
			node.setAttribute("aria-label", slot.label);
			node.tabIndex = -1;
			strip.append(node);
			cells.push({
				node,
				tip: slot.tip
			});
		}
		grid.push(cells);
		item.append(name, strip);
		list.append(item);
	}
	grid[0][0].node.tabIndex = 0;
	const ends = htmlEl("li", "tmt-ui-chart-strip-row");
	ends.setAttribute("aria-hidden", "true");
	const span = htmlEl("div", "tmt-ui-chart-strip-ends");
	span.append(htmlEl("span", void 0, options.ends[0]), htmlEl("span", void 0, options.ends[1]));
	ends.append(htmlEl("span", "tmt-ui-chart-strip-label"), span);
	list.append(ends);
	const find = (target) => {
		const node = target?.closest(".tmt-ui-chart-slot");
		if (!node) return void 0;
		for (const [r, cells] of grid.entries()) {
			const c = cells.findIndex((cell) => cell.node === node);
			if (c >= 0) return {
				r,
				c
			};
		}
	};
	function show(r, c) {
		const cell = grid[r][c];
		tip.replaceChildren(htmlEl("div", "tmt-ui-chart-tip-title", cell.tip.seat), htmlEl("div", void 0, cell.tip.range), htmlEl("div", "tmt-ui-chart-legend-value", cell.tip.value));
		tip.hidden = false;
		const box = cell.node.getBoundingClientRect();
		const at = frame.getBoundingClientRect();
		const left = box.left - at.left + box.width / 2 - tip.offsetWidth / 2;
		tip.style.left = `${Math.max(0, Math.min(left, at.width - tip.offsetWidth))}px`;
		tip.style.top = `${box.bottom - at.top + 6}px`;
	}
	const hide = () => tip.hidden = true;
	list.addEventListener("pointerover", (event) => {
		const at = find(event.target);
		if (at) show(at.r, at.c);
	});
	list.addEventListener("pointerleave", hide);
	list.addEventListener("focusin", (event) => {
		const at = find(event.target);
		if (at) show(at.r, at.c);
	});
	list.addEventListener("focusout", hide);
	list.addEventListener("keydown", (event) => {
		const at = find(event.target);
		if (!at) return;
		const row = grid[at.r];
		const to = event.key === "ArrowRight" ? {
			r: at.r,
			c: Math.min(row.length - 1, at.c + 1)
		} : event.key === "ArrowLeft" ? {
			r: at.r,
			c: Math.max(0, at.c - 1)
		} : event.key === "ArrowDown" ? {
			r: Math.min(grid.length - 1, at.r + 1),
			c: at.c
		} : event.key === "ArrowUp" ? {
			r: Math.max(0, at.r - 1),
			c: at.c
		} : event.key === "Home" ? {
			r: at.r,
			c: 0
		} : event.key === "End" ? {
			r: at.r,
			c: row.length - 1
		} : void 0;
		if (!to) return;
		event.preventDefault();
		grid[at.r][at.c].node.tabIndex = -1;
		const next = grid[to.r][to.c].node;
		next.tabIndex = 0;
		next.focus();
	});
	frame.append(list, tip);
	figure.append(frame);
	if (options.legend !== false) figure.append(stripsLegend());
	return figure;
}
//#endregion
//#region src/page-loading.ts
/** A Remote page's wait for its first read: the busy mark, centred where the read fills. */
function pageLoading() {
	const row = document.createElement("p");
	row.className = "tmt-page-loading";
	row.setAttribute("role", "status");
	row.setAttribute("aria-label", "Loading");
	row.append(createBusyMark(document));
	return row;
}
//#endregion
//#region src/owner-reads.ts
/**
* The owner browser reads (contracts/remote-channel-v1.md, Owner browser reads): read-only door GETs
* under the owner session, siblings of the composed shell's `x/-/shell`. One client for every page
* that shows them (Ops, Digest), and the refresh timer that runs only while the route is visible.
*/
/** The reasons the contract names; a client reads any other reason as `unknown`. */
var OWNER_READ_REASONS = [
	"not_installed",
	"core_unavailable",
	"timeout",
	"read_failed",
	"too_large",
	"not_sampled"
];
var UNREACHABLE = { state: "unreachable" };
/** One answer document checked against the contract's envelope; anything else is unreachable. */
function ownerAnswer(document) {
	if (typeof document !== "object" || document === null || Array.isArray(document)) return UNREACHABLE;
	const { version, state, observedAtMs, reason, ...payload } = document;
	if (version !== 1 || !Number.isSafeInteger(observedAtMs) || observedAtMs <= 0) return UNREACHABLE;
	const at = observedAtMs;
	if (state === "ok") return {
		state,
		observedAtMs: at,
		payload
	};
	if (state !== "unavailable") return UNREACHABLE;
	return {
		state,
		observedAtMs: at,
		reason: OWNER_READ_REASONS.find((name) => name === reason) ?? "unknown"
	};
}
var address;
/** Without an owner session, a pairing kept in this browser opens one again (a door restart ends
* them); `no-session` when there is no pairing to use, else `unreachable` (try again later). */
async function reopenOnce$1(signal) {
	try {
		const { reopenSession } = await import("./remote-v1.js");
		await reopenSession(void 0, { signal });
		return "reopened";
	} catch (error) {
		const reason = error?.reason;
		return reason === "unpaired" || reason === "revoked" || reason === "expired" ? "no-session" : "unreachable";
	}
}
/** The reopen in flight: reads that meet the same door restart together wait on this one. */
var reopening;
/** Join the reopen in flight, or start one. It stops only once every read waiting on it left; a
* read whose own signal aborts leaves at once as `unreachable`. */
function reopen(signal) {
	if (reopening === void 0) {
		const stop = new AbortController();
		const shared = {
			result: reopenOnce$1(stop.signal),
			waiting: 0,
			stop
		};
		shared.result.finally(() => {
			if (reopening === shared) reopening = void 0;
		});
		reopening = shared;
	}
	const shared = reopening;
	shared.waiting += 1;
	return new Promise((resolve) => {
		const leave = () => {
			shared.waiting -= 1;
			if (shared.waiting === 0) {
				shared.stop.abort();
				if (reopening === shared) reopening = void 0;
			}
			resolve("unreachable");
		};
		if (signal.aborted) return leave();
		signal.addEventListener("abort", leave, { once: true });
		shared.result.then((result) => {
			signal.removeEventListener("abort", leave);
			resolve(result);
		});
	});
}
async function readOnce(url, signal) {
	try {
		return await fetch(url, {
			credentials: "same-origin",
			cache: "no-store",
			signal
		});
	} catch {
		return;
	}
}
/**
* Read `<address>/x/-/<name>` once. The mount address is found once per page and found again
* after a read that had none. The door answers a request without a live owner session with its
* generic 404: the read then reopens the session from this browser's pairing and reads again, so
* only a browser with no pairing to use is told it has no session (#4018). Reads that meet the
* same restart share one reopen.
*/
async function ownerRead(name, signal) {
	address ??= shellAddress();
	const base = await address;
	if (base === void 0) {
		address = void 0;
		return { state: "no-session" };
	}
	const url = `${base}/x/-/${name}`;
	let response = await readOnce(url, signal);
	if (response?.status === 404) {
		const reopened = await reopen(signal);
		if (reopened !== "reopened") return { state: reopened };
		response = await readOnce(url, signal);
	}
	if (response?.status !== 200) return UNREACHABLE;
	try {
		return ownerAnswer(await response.json());
	} catch {
		return UNREACHABLE;
	}
}
/** The longest wait after failures: the cadence doubles per failed read up to this many times. */
var BACKOFF_STEPS = 3;
/**
* Run `tick` now and every `intervalMs` while the page is visible, until `signal` aborts (the route
* unmounts). A hidden page reads nothing; showing it again reads at once. A tick that returns false
* (a failed read) doubles the wait, up to eight times the cadence, and the next success resets it.
*/
function whileVisible(intervalMs, signal, tick) {
	let timer;
	let failures = 0;
	let running = false;
	const schedule = () => {
		clearTimeout(timer);
		if (signal.aborted || document.visibilityState !== "visible") return;
		timer = setTimeout(run, intervalMs * 2 ** Math.min(failures, BACKOFF_STEPS));
	};
	const run = async () => {
		clearTimeout(timer);
		if (running || signal.aborted || document.visibilityState !== "visible") return;
		running = true;
		try {
			failures = await tick() ? 0 : failures + 1;
		} catch {
			failures += 1;
		} finally {
			running = false;
		}
		schedule();
	};
	document.addEventListener("visibilitychange", () => document.visibilityState === "visible" ? void run() : clearTimeout(timer), { signal });
	signal.addEventListener("abort", () => clearTimeout(timer), { once: true });
	run();
}
//#endregion
//#region src/digest-source.ts
/**
* What the Digest web view (`/digest/`) reads: the usage report the terminal Digest shows. The
* types follow the public document of `tmt digest usage report --json` (digest `usage-v1.md`) and
* the exchange flow of `tmt x flow --json`; the page depends only on `DigestSource`. The product
* source reads the owner's browser reads (`x/-/digest/<range>` and `x/-/limits`, contracts/
* remote-channel-v1.md); the seeded sample lives in `digest-mock.ts` for tests, the gallery and
* dev only.
*/
var USAGE_RANGES = [
	"24h",
	"7d",
	"30d"
];
/** One plain sentence per reason, each saying what the person can do. */
var REASONS$1 = {
	not_installed: "Digest is not installed on your machine. Run `tmt extension install digest`, then reload this page.",
	core_unavailable: "TMT on your machine did not answer. This page tries again shortly.",
	timeout: "Your machine took too long to read usage. This page tries again shortly.",
	read_failed: "Your machine could not read usage. Run `tmt digest usage report` in a terminal to see it.",
	too_large: "The usage report is too large for the browser. Run `tmt digest usage report` in a terminal to see it.",
	not_sampled: "Your machine could not read usage. Run `tmt digest usage report` in a terminal to see it.",
	unknown: "Usage is not available right now. Run `tmt digest usage report` in a terminal to see it.",
	"no-session": "This browser has no owner session for your machine. Pair it from Remote to see your usage.",
	unreachable: "Your machine did not answer. This page tries again shortly."
};
/** The model family a driver's weekly window belongs to, as the report names it on its seats. */
var DRIVER_LINES = {
	claude: "claude",
	codex: "gpt"
};
/**
* The percent-left history of the current weekly window per provider. The cache keeps 8 days, so
* its points can span two windows: only those of the newest window (its reset time) are drawn.
*/
function limitHistoryFrom(answer) {
	if (answer.state !== "ok") return void 0;
	const histories = answer.payload.limitHistory;
	if (!Array.isArray(histories)) return void 0;
	const isPoint = (point) => Array.isArray(point) && point.length === 3 && point.every((value) => Number.isFinite(value));
	return histories.flatMap((entry) => {
		const history = entry;
		if (typeof history?.provider !== "string" || !Number.isFinite(history.windowMinutes) || !Array.isArray(history.points)) return [];
		const points = history.points.filter(isPoint);
		const last = points.at(-1);
		if (!last) return [];
		return [{
			line: DRIVER_LINES[history.provider] ?? history.provider,
			windowMinutes: history.windowMinutes,
			resetsAtMs: last[2],
			points: points.filter((point) => point[2] === last[2]).map(([atMs, percentLeft]) => ({
				atMs,
				percentLeft
			}))
		}];
	});
}
/**
* The report from the two owner reads. An unavailable Digest read is the page's empty state with
* its reason; an unavailable limits read only leaves the history out (the latest window still
* shows), since `limits` needs Ops and a machine whose board never ran has no samples.
*/
function reportFromAnswers(range, digest, limits) {
	if (digest.state !== "ok") return {
		range,
		sample: false,
		unavailable: REASONS$1[digest.state === "unavailable" ? digest.reason : digest.state]
	};
	const document = digest.payload.report;
	if (digest.payload.since !== range || document?.since !== range) return {
		range,
		sample: false,
		unavailable: REASONS$1.unknown
	};
	const limitHistory = limitHistoryFrom(limits);
	return {
		range,
		sample: false,
		document: limitHistory ? {
			...document,
			limitHistory
		} : document
	};
}
/** The source the page reads: the owner's usage report and weekly-limit history. */
var digestSource = { async report(range, signal = new AbortController().signal) {
	const [digest, limits] = await Promise.all([ownerRead(`digest/${range}`, signal), ownerRead("limits", signal)]);
	return reportFromAnswers(range, digest, limits);
} };
//#endregion
//#region src/digest-view.ts
var HOUR = 36e5;
var DAY = 24 * HOUR;
/** Input plus output; cached input is part of input and is never added. */
var tokensOf = (point) => point.input + point.output;
function seatName(seat) {
	const base = seat.name ?? seat.id.slice(0, 8);
	return seat.retired ? `${base} (retired)` : base;
}
function stamp(ms, range, zone) {
	const parts = new Intl.DateTimeFormat("en-GB", {
		timeZone: zone,
		month: "2-digit",
		day: "2-digit",
		hour: "2-digit",
		minute: "2-digit",
		hourCycle: "h23"
	}).formatToParts(new Date(ms));
	const at = (type) => parts.find((part) => part.type === type).value;
	return range === "24h" ? `${at("hour")}:${at("minute")}` : `${at("month")}-${at("day")}`;
}
/** Tokens per bucket for every seat, on the union of the seats' bucket starts. */
function tokenSeries(doc, zone) {
	const starts = [...new Set(doc.usage.seats.flatMap((seat) => seat.series.map((point) => point.fromMs)))].sort((a, b) => a - b);
	return {
		x: starts.map((at) => stamp(at, doc.since, zone)),
		layers: doc.usage.seats.map((seat) => {
			const byStart = new Map(seat.series.map((point) => [point.fromMs, point]));
			return {
				key: seat.id,
				label: seatName(seat),
				values: starts.map((at) => {
					const point = byStart.get(at);
					return point ? tokensOf(point) : null;
				})
			};
		}),
		partial: doc.usage.seats.some((seat) => seat.series.some((point) => !point.complete))
	};
}
/** Seats by tokens in the window, largest first (ties by name). */
function rankRows(doc) {
	const rows = doc.usage.seats.map((seat) => ({
		id: seat.id,
		name: seatName(seat),
		tokens: tokensOf(seat.totals),
		partial: !seat.coverage.complete
	}));
	const total = rows.reduce((sum, row) => sum + row.tokens, 0);
	return rows.map((row) => ({
		...row,
		share: total ? Math.round(row.tokens / total * 100) : 0
	})).sort((a, b) => b.tokens - a.tokens || a.name.localeCompare(b.name));
}
var COUNTED = /* @__PURE__ */ new Set(["talk", "reply"]);
/**
* Who talks to whom: only talk and reply count (an ack or a result is not a conversation),
* seats ranked by messages sent plus received, the busiest `limit` kept. Null without a flow
* read; a pair with no message is a known none (0).
*/
function exchange(doc, limit = 8) {
	const flow = doc.exchange;
	if (!flow) return null;
	const labels = /* @__PURE__ */ new Map();
	for (const seat of doc.usage.seats) labels.set(seat.id, seatName(seat));
	const sent = /* @__PURE__ */ new Map();
	const pairs = /* @__PURE__ */ new Map();
	for (const pair of flow.pairs) {
		const { sender, recipient } = pair;
		if (!COUNTED.has(pair.kind) || !sender.id || !recipient.id || sender.id === recipient.id) continue;
		for (const party of [sender, recipient]) if (!labels.has(party.id)) labels.set(party.id, seatName({
			...party,
			id: party.id
		}));
		pairs.set(`${sender.id}>${recipient.id}`, (pairs.get(`${sender.id}>${recipient.id}`) ?? 0) + pair.count);
		for (const id of [sender.id, recipient.id]) sent.set(id, (sent.get(id) ?? 0) + pair.count);
	}
	return {
		participants: [...labels].map(([id, label]) => ({
			id,
			label
		})).sort((a, b) => (sent.get(b.id) ?? 0) - (sent.get(a.id) ?? 0) || a.label.localeCompare(b.label)).slice(0, limit),
		value: (from, to) => pairs.get(`${from}>${to}`) ?? 0
	};
}
var SLOTS = {
	"24h": {
		count: 24,
		stepMs: 36e5,
		slot: "hour"
	},
	"7d": {
		count: 7,
		stepMs: 864e5,
		slot: "day"
	},
	"30d": {
		count: 30,
		stepMs: 864e5,
		slot: "day"
	}
};
/**
* One slot per seat across the range, at the resolution the report carries: hourly for 24 h,
* daily for 7d and 30d. The slots end at the newest bucket; one with no bucket is not observed
* (null), never zero.
*/
function activity(doc) {
	const shape = SLOTS[doc.since];
	if (!shape) return null;
	const latest = Math.max(...doc.usage.seats.flatMap((seat) => seat.series.map((point) => point.fromMs)));
	if (!Number.isFinite(latest)) return null;
	const starts = Array.from({ length: shape.count }, (_, at) => latest - (shape.count - 1 - at) * shape.stepMs);
	return {
		starts,
		slot: shape.slot,
		rows: doc.usage.seats.map((seat) => {
			const byStart = new Map(seat.series.map((point) => [point.fromMs, point]));
			return {
				id: seat.id,
				label: seatName(seat),
				slots: starts.map((at) => {
					const point = byStart.get(at);
					return point ? tokensOf(point) : null;
				})
			};
		})
	};
}
var LINE_LABELS = {
	claude: "Claude",
	gpt: "OpenAI"
};
/** A provider keeps one hue across its meter and its chart: orange for Claude, aqua for OpenAI. */
function lineStyle(line) {
	return line === "claude" ? 1 : line === "gpt" ? 2 : 0;
}
/** The weekly windows, one row each: how much is left and whether the pace is even. */
function allowance(doc) {
	const lines = new Map(doc.usage.seats.map((seat) => [seat.id, seat.lines[0] ?? ""]));
	return doc.limits.filter((limit) => limit.windowMinutes === 10080).map((limit) => {
		const line = limit.seats.map((id) => lines.get(id)).find(Boolean) ?? "";
		return {
			line,
			provider: LINE_LABELS[line] ?? (line ? line[0].toUpperCase() + line.slice(1) : "Account"),
			leftPercent: Math.round((100 - limit.usedPercent) * 10) / 10,
			elapsedPercent: limit.elapsedPercent,
			paceDeltaPoints: limit.paceDeltaPoints,
			resetsAtMs: limit.resetsAtMs,
			stale: limit.stale
		};
	}).sort((a, b) => a.provider.localeCompare(b.provider) || a.resetsAtMs - b.resetsAtMs);
}
/** Weekly percent-left history charts, one per provider; none without the history field. */
function historyCharts(doc, zone) {
	return (doc.limitHistory ?? []).filter((history) => history.points.length > 1).map((history) => {
		const x = history.points.map((point) => stamp(point.atMs, "7d", zone));
		const window = history.windowMinutes * 6e4;
		return {
			title: "Weekly percent left",
			kind: "line",
			style: lineStyle(history.line),
			x,
			series: [{
				key: history.line,
				label: "left",
				values: history.points.map((point) => point.percentLeft)
			}],
			reference: {
				label: "even pace",
				values: history.points.map((point) => Math.max(0, Math.min(100, 100 * (history.resetsAtMs - point.atMs) / window)))
			},
			max: 100,
			format: (value) => `${Math.round(value)}%`
		};
	});
}
/** One entry per weekly window: its numbers beside its history chart, matched by provider. */
function providers(doc, zone) {
	const charts = new Map(historyCharts(doc, zone).map((chart) => [chart.series[0].key, chart]));
	return allowance(doc).map((row) => ({
		...row,
		style: lineStyle(row.line),
		chart: charts.get(row.line)
	}));
}
/** Days between two instants, for the page's "resets in" words. */
function resetsIn(resetsAtMs, nowMs) {
	const ms = resetsAtMs - nowMs;
	if (ms <= 0) return "now";
	if (ms >= DAY) return `${Math.floor(ms / DAY)}d ${Math.floor(ms % DAY / HOUR)}h`;
	return `${Math.max(1, Math.floor(ms / HOUR))}h`;
}
//#endregion
//#region src/not-available.ts
/**
* The empty state of a page whose browser read does not exist yet. It tells the person what they
* can do instead: the terminal command that shows the same facts. Product builds show this, never
* sample values; mocks stay in tests, the gallery and the dev preview.
*/
function notInBrowser(page, command) {
	return `${page} is not in the browser yet. Run \`${command}\` in a terminal.`;
}
/** Show `text` in `note`, each `command` in backticks as code. */
function notAvailable(note, text) {
	note.hidden = false;
	if (!text.includes("`")) {
		note.textContent = text;
		return;
	}
	note.replaceChildren(...text.split("`").map((part, index) => {
		if (index % 2 === 0) return part;
		const code = document.createElement("code");
		code.textContent = part;
		return code;
	}));
}
//#endregion
//#region src/digest-page.ts
/**
* The Digest web view: the usage report as cards of charts and tables. The arrangement is the
* team's usage at a glance: provider allowance (meter, weekly history, pace), who burns (tokens
* over time beside the ranking), activity by seat, then who talks to whom beside the data the
* read does not carry yet and the method. It depends only on `DigestSource`; the product source
* says "not in the browser yet" until Core serves the read (#4325), and a sample source is only
* ever passed in by tests, the gallery and dev.
*/
/** The terminal Digest's own cadence: the usage read refreshes once a minute. */
var REFRESH_MS$1 = 6e4;
/** Distinct series a chart draws; the rest fold into one neutral `Other`. */
var SERIES_KEPT = 5;
/** The neutral palette step of the folded `Other` layer (`tmt-ui-chart-s7`). */
var OTHER_STEP = 7;
function el(name, className, text) {
	const node = document.createElement(name);
	if (className) node.className = className;
	if (text !== void 0) node.textContent = text;
	return node;
}
/** A flat sheet with a sentence-case title and an optional right-hand slot in its head. */
function card(id, title, body, options = {}) {
	const node = el("section", options.className ? `digest-card ${options.className}` : "digest-card");
	node.id = id;
	node.setAttribute("aria-labelledby", `${id}-title`);
	const heading = el("h2", "digest-card-title", title);
	heading.id = `${id}-title`;
	const head = el("div", "digest-card-head");
	head.append(heading);
	if (options.actions) head.append(options.actions);
	node.append(head, ...body);
	return node;
}
function row$2(className, ...cards) {
	const node = el("div", `digest-row ${className}`);
	node.append(...cards);
	return node;
}
function table(head, rows) {
	const node = el("table", "usage-table");
	const header = el("tr");
	for (const text of head) {
		const cell = el("th", void 0, text);
		cell.scope = "col";
		header.append(cell);
	}
	const thead = el("thead");
	thead.append(header);
	node.append(thead);
	const body = el("tbody");
	for (const [first, ...rest] of rows) {
		const line = el("tr");
		const name = el("th");
		name.scope = "row";
		name.append(first);
		line.append(name, ...rest.map((value) => {
			const cell = el("td");
			cell.append(value);
			return cell;
		}));
		body.append(line);
	}
	node.append(body);
	return node;
}
/** A segmented control of buttons; `aria-pressed` marks the active one. */
function segments(label, options, initial, change) {
	const group = el("div", "digest-segments");
	group.setAttribute("role", "group");
	group.setAttribute("aria-label", label);
	const buttons = options.map((option) => {
		const button = el("button", "digest-segment", option.text);
		button.type = "button";
		button.addEventListener("click", () => {
			sync(option.value);
			change(option.value);
		});
		return {
			button,
			value: option.value
		};
	});
	const sync = (active) => {
		for (const { button, value } of buttons) button.setAttribute("aria-pressed", String(value === active));
	};
	sync(initial);
	group.append(...buttons.map(({ button }) => button));
	return group;
}
function providerCard(view, nowMs) {
	const resets = view.stale ? "stale" : resetsIn(view.resetsAtMs, nowMs);
	const meter = el("div", "digest-meter");
	meter.setAttribute("role", "img");
	meter.setAttribute("aria-label", `${view.leftPercent}% of the weekly allowance left`);
	const fill = el("div", `digest-meter-fill tmt-ui-chart-s${view.style}`);
	fill.style.setProperty("--digest-fill", String(Math.max(0, Math.min(100, view.leftPercent))));
	meter.append(fill);
	const figure = el("div", "digest-provider-figure");
	figure.append(el("p", "digest-big", `${view.leftPercent}%`), el("p", "digest-big-label", "allowance left"), meter);
	const history = view.chart ? createSeriesChart(view.chart) : el("p", "usage-note", "The weekly history is not served yet; this shows the latest window.");
	const stat = (label, value) => {
		const cell = el("div", "digest-stat");
		cell.append(el("dt", void 0, label), el("dd", void 0, value));
		return cell;
	};
	const stats = el("dl", "digest-stats");
	stats.append(stat("Elapsed", view.elapsedPercent === null ? "—" : `${Math.round(view.elapsedPercent)}%`), stat("Pace", view.paceDeltaPoints === null ? "—" : `${view.paceDeltaPoints > 0 ? "+" : ""}${view.paceDeltaPoints.toFixed(1)} pts`), stat("Resets in", resets));
	const body = el("div", "digest-provider-body");
	body.append(figure, history);
	return card(`digest-provider-${view.line || "account"}`, view.provider, [body, stats], {
		actions: el("span", "digest-card-meta", `Resets in ${resets}`),
		className: "digest-provider"
	});
}
function allowanceRow(doc) {
	const views = providers(doc);
	const node = views.length ? row$2("digest-row-providers", ...views.map((view) => providerCard(view, doc.observedAtMs))) : row$2("digest-row-providers", card("digest-provider-none", "Allowance", [el("p", "usage-note", "No weekly window has been observed yet.")]));
	node.id = "digest-allowance";
	return node;
}
function tokensCard(doc, scale) {
	const data = tokenSeries(doc);
	const body = el("div", "digest-card-body");
	const host = el("div");
	const draw = () => {
		host.replaceChildren(createSeriesChart({
			title: scale.share ? "Share of tokens by seat" : "Tokens by seat",
			kind: scale.share ? "share" : "stack",
			x: data.x,
			series: topLayers(data.layers, SERIES_KEPT),
			empty: "No token observations in this range."
		}));
	};
	draw();
	body.append(host);
	if (data.partial) body.append(el("p", "usage-note", "Some buckets are incomplete: they are lower bounds, not errors."));
	const control = segments("Scale", [{
		value: "absolute",
		text: "Absolute"
	}, {
		value: "share",
		text: "Share"
	}], scale.share ? "share" : "absolute", (value) => {
		scale.share = value === "share";
		draw();
	});
	return card("digest-tokens", "Token usage", [body], { actions: control });
}
function rankingCard(doc) {
	const rows = rankRows(doc);
	const hue = new Map(topLayers(tokenSeries(doc).layers, SERIES_KEPT).filter((layer) => layer.key !== "other").map((layer, at) => [layer.key, at]));
	const largest = Math.max(1, ...rows.map((entry) => entry.tokens));
	const bar = (id, tokens) => {
		const node = el("span", `digest-bar tmt-ui-chart-s${hue.get(id) ?? OTHER_STEP}`);
		node.style.setProperty("--digest-bar", String(Math.round(tokens / largest * 100)));
		node.setAttribute("aria-hidden", "true");
		return node;
	};
	return card("digest-ranking", `Last ${doc.since} by seat`, rows.length ? [table([
		"Seat",
		"",
		"Tokens",
		"Share"
	], rows.map((entry) => [
		entry.partial ? `${entry.name} (partial)` : entry.name,
		bar(entry.id, entry.tokens),
		compactCount(entry.tokens),
		`${entry.share}%`
	]))] : [el("p", "usage-note", "No seat has token observations in this range.")]);
}
function activityCard(doc) {
	const view = activity(doc);
	if (!view) return void 0;
	const clock = (at) => new Date(at).toLocaleTimeString("en-GB", {
		hour: "2-digit",
		minute: "2-digit"
	});
	const day = (at) => new Date(at).toLocaleDateString("en-GB", {
		weekday: "short",
		day: "numeric",
		month: "short"
	});
	const hourly = view.slot === "hour";
	const range = (at) => {
		const start = view.starts[at];
		return hourly ? `${clock(start)}–${clock(start + 36e5)}` : day(start);
	};
	return card("digest-activity", "Activity by seat", [createStrips({
		title: hourly ? "Tokens per hour, oldest on the left" : "Tokens per day, oldest on the left",
		rows: view.rows,
		slotCount: view.starts.length,
		slotRange: range,
		ends: hourly ? ["24 h ago", "now"] : [`${view.starts.length} days ago`, "today"],
		legend: false
	}), el("p", "usage-note", "Token flow only. Board row state and age, pane evidence and what each seat waits on are not in the usage read yet.")], { actions: stripsLegend() });
}
function exchangeCard(doc) {
	const flow = exchange(doc);
	return card("digest-exchange", "Messages between seats", [flow ? createMatrix({
		title: `Messages, last ${doc.since}`,
		participants: flow.participants,
		value: flow.value,
		sum: true
	}) : el("p", "usage-note", "Message traffic is not available in this read.")]);
}
/** The sources the usage read does not carry yet, one line each, with the method below. */
function coverageCard() {
	const gaps = [
		["Throughput", "merged pull requests per provider and publishes in flight (GitHub, from the PR Agent lines)"],
		["Gates and queue", "green-not-armed and review waits, merge-queue entries and merge-group failures (GitHub)"],
		["Cost per merged PR", "merged pull requests joined to the allowance points burned in the same window"],
		["Machine", "load, free disk and the biggest worktrees with their owners"]
	];
	const list = el("dl", "digest-gaps");
	for (const [term, source] of gaps) {
		const line = el("div", "digest-gap");
		line.append(el("dt", void 0, term), el("dd", "digest-badge", "Not connected"), el("dd", "digest-gap-source", source));
		list.append(line);
	}
	const method = el("details", "digest-method");
	method.id = "digest-method";
	method.append(el("summary", void 0, "Method & caveats"), ...[
		"Tokens are input plus output as Core reports them; cached input is part of input and is not counted again.",
		"An incomplete bucket is a lower bound. A seat with no evidence for part of the range is marked partial, never shown as zero.",
		"Allowance windows are the account percent as the provider reports it, in whole percent; usage on other machines is not seen.",
		"Message counts include talk and reply only."
	].map((text) => el("p", "usage-note", text)));
	return card("digest-missing", "Data coverage", [list, method]);
}
/** What a report shows, without the read's own clock: a refresh that changed nothing repaints nothing. */
function fingerprint(report) {
	return JSON.stringify({
		...report,
		document: report.document && {
			...report.document,
			observedAtMs: 0
		}
	});
}
function digestPage(source = digestSource, signal) {
	const tabs = document.getElementById("usage-ranges");
	const panel = document.getElementById("digest-sections");
	const note = document.getElementById("usage-note");
	const badge = document.getElementById("usage-badge");
	if (!tabs || !panel || !note || !badge) return;
	let current = USAGE_RANGES[0];
	let generation = 0;
	let shown = "";
	let good;
	const scale = { share: false };
	const buttons = USAGE_RANGES.map((range) => {
		const button = el("button", "digest-segment", range);
		button.type = "button";
		button.setAttribute("role", "tab");
		button.setAttribute("aria-controls", "usage-panel");
		button.dataset.range = range;
		button.addEventListener("click", () => void show(range));
		button.addEventListener("keydown", (event) => {
			const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
			if (!step) return;
			event.preventDefault();
			const range = USAGE_RANGES[(USAGE_RANGES.indexOf(current) + step + USAGE_RANGES.length) % USAGE_RANGES.length];
			show(range).then(() => buttons[USAGE_RANGES.indexOf(range)].focus());
		});
		return button;
	});
	tabs.replaceChildren(...buttons);
	function paint(report) {
		shown = fingerprint(report);
		good = void 0;
		badge.hidden = true;
		badge.textContent = "";
		if (report.unavailable || !report.document) {
			panel.replaceChildren();
			return notAvailable(note, report.unavailable ?? "Usage is not available.");
		}
		good = report;
		const doc = report.document;
		const activityBlock = activityCard(doc);
		panel.replaceChildren(allowanceRow(doc), row$2("digest-row-burn", tokensCard(doc, scale), rankingCard(doc)), ...activityBlock ? [activityBlock] : [], row$2("digest-row-coverage", exchangeCard(doc), coverageCard()));
		if (report.sample) badge.textContent = "Sample data";
		badge.hidden = !report.sample;
		ordinaryNote(report);
	}
	/** The note a good report carries: the sample's preview line, else nothing. */
	function ordinaryNote(report) {
		if (report.sample) notAvailable(note, "Usage preview. Real numbers: `tmt digest usage report`.");
		else {
			note.replaceChildren();
			note.hidden = true;
		}
	}
	/**
	* A refresh that could not read keeps the last good report on screen and says why in a small
	* note; only a first read with nothing to show empties the panel. The next good read clears it.
	*/
	function refreshed(report) {
		if (report.unavailable || !report.document) {
			if (!good?.document) return paint(report);
			const at = new Date(good.document.observedAtMs).toLocaleTimeString("en-GB", {
				hour: "2-digit",
				minute: "2-digit"
			});
			return notAvailable(note, `${report.unavailable ?? "Usage is not available."} Showing the last report, read at ${at}.`);
		}
		if (fingerprint(report) !== shown) return paint(report);
		ordinaryNote(report);
	}
	async function show(range) {
		current = range;
		const mine = ++generation;
		for (const button of buttons) {
			const selected = button.dataset.range === range;
			button.setAttribute("aria-selected", String(selected));
			button.tabIndex = selected ? 0 : -1;
		}
		panel.replaceChildren(pageLoading());
		const report = await source.report(range, signal);
		if (mine !== generation || signal?.aborted) return;
		paint(report);
	}
	show(current);
	let first = true;
	if (signal) whileVisible(REFRESH_MS$1, signal, async () => {
		if (first) {
			first = false;
			return true;
		}
		const range = current;
		const mine = generation;
		const report = await source.report(range, signal);
		if (signal.aborted || range !== current || mine !== generation) return true;
		refreshed(report);
		return !report.unavailable;
	});
}
//#endregion
//#region src/module-entry.ts
/**
* Where the shell may load a module's code from. A composed shell answer names a module's mount
* (`rail.path`) and optionally its `entry`; the shell imports the entry only when both are paths
* of this same origin under `/r/<prefix>/x/<module>/`, compared by path segment, naming a script
* (`.js`). Anything else is
* refused, and the module's own link is the fallback.
*/
/** An entry has no escapes, backslashes, query, fragment, control characters or dot segments. */
function plain(entry) {
	if (/[\\%?#\u0000-\u001f\u007f]/.test(entry)) return false;
	return entry.split("/").every((segment) => segment !== "." && segment !== "..");
}
/**
* Confine a described module to its mount; `here` is the page's URL, the base of relative paths.
* A module that names no `entry` loads `fallback` (an older Colab names none); without either it
* has no entry and only its link.
*/
function moduleLoad(rail, entry, here, fallback) {
	let page;
	let mount;
	try {
		page = new URL(here);
		mount = new URL(rail, page);
	} catch {
		return {};
	}
	const link = mount.pathname.endsWith("/") ? mount.pathname : `${mount.pathname}/`;
	if (mount.origin !== page.origin || mount.search || mount.hash || !moduleMount(link)) return {};
	const wanted = entry ?? fallback;
	if (wanted === void 0) return { link };
	if (!plain(wanted)) return { link };
	let target;
	try {
		target = new URL(wanted, new URL(link, page));
	} catch {
		return { link };
	}
	return target.origin === page.origin && target.pathname.startsWith(link) && target.pathname.length > link.length && target.pathname.endsWith(".js") && !target.pathname.includes("//") && !target.search && !target.hash ? {
		link,
		entry: target.href
	} : { link };
}
//#endregion
//#region src/ops-source.ts
/**
* What the Ops web view (`/ops/`) reads: the owner's squad board through the owner browser read
* `x/-/ops` (contracts/remote-channel-v1.md, Owner browser reads), read-only. The page depends only
* on `OpsSource`; this module turns the read's allowlisted projection into the page's rows.
*/
var AGENT_STATES = {
	working: {
		mark: "●",
		word: "Working",
		tone: "working"
	},
	idle: {
		mark: "◌",
		word: "Idle",
		tone: "neutral"
	},
	"needs-you": {
		mark: "◆",
		word: "Needs you",
		tone: "waiting"
	},
	blocked: {
		mark: "✗",
		word: "Blocked",
		tone: "blocked"
	},
	offline: {
		mark: "○",
		word: "Offline",
		tone: "neutral"
	},
	"idle-alert": {
		mark: "◷",
		word: "Idle alert",
		tone: "neutral"
	},
	unknown: {
		mark: "?",
		word: "Unknown",
		tone: "neutral"
	}
};
/** A row's mark: what it owes the person first, then the board's colour token, then activity. */
function agentState(agent) {
	if (agent.presence === "offline") return "offline";
	if (agent.pending || agent.waitingOnYou > 0) return "needs-you";
	if (agent.stateTone === "blocked") return "blocked";
	if (agent.stateTone === "working") return "working";
	if (agent.stateTone === "dim") return "idle";
	const activity = agent.activity?.state;
	return activity === "working" || activity === "idle" ? activity : "unknown";
}
/** One plain sentence per reason, each saying what the person can do. */
var REASONS = {
	not_installed: "Ops is not installed on your machine. Run `tmt extension install ops`, then reload this page.",
	core_unavailable: "TMT on your machine did not answer. This page tries again shortly.",
	timeout: "Your machine took too long to read the board. This page tries again shortly.",
	read_failed: "Your machine could not read the board. Run `tmt ops ui` in a terminal to see it.",
	too_large: "The board is too large for the browser. Run `tmt ops ui` in a terminal to see it.",
	not_sampled: "Your machine could not read the board. Run `tmt ops ui` in a terminal to see it.",
	unknown: "The board is not available right now. Run `tmt ops ui` in a terminal to see it.",
	"no-session": "This browser has no owner session for your machine. Pair it from Remote to see the board.",
	unreachable: "Your machine did not answer. This page tries again shortly."
};
function unavailableBoard(reason) {
	return {
		agents: [],
		squads: [],
		schedules: [],
		truncated: false,
		unavailable: REASONS[reason]
	};
}
/** The page's board from one `ok` payload; a payload off the contract is a failed read. */
function opsBoard(payload) {
	if (!Array.isArray(payload.squads) || !Array.isArray(payload.agents)) return unavailableBoard("read_failed");
	return {
		squads: payload.squads.map((squad) => ({
			name: squad.name,
			...squad.lead ? { lead: squad.lead.name } : {},
			...squad.idleAlertMs ? { idleAlertMinutes: Math.round(squad.idleAlertMs / 6e4) } : {}
		})),
		agents: payload.agents.map((agent) => {
			const state = agentState(agent);
			return {
				name: agent.name,
				squad: agent.squad,
				role: agent.role,
				state,
				...agent.state && agent.state.toLowerCase() !== AGENT_STATES[state].word.toLowerCase() ? { stateText: agent.state } : {},
				...agent.task ? { task: agent.task } : {},
				...agent.next ? { next: agent.next } : {},
				...agent.pending ? { pending: agent.pending } : {},
				...agent.model ? { model: agent.model } : {},
				waitingOnYou: agent.waitingOnYou ?? 0
			};
		}),
		schedules: Array.isArray(payload.timers) ? payload.timers.map((timer) => ({
			id: timer.id,
			squad: timer.squad,
			when: timer.scheduleText,
			what: timer.what ?? "",
			...timer.owner ? { owner: timer.owner } : {}
		})) : null,
		truncated: payload.truncated === true
	};
}
/** The board from an owner read's answer. */
function boardFromAnswer(answer) {
	if (answer.state === "ok") return opsBoard(answer.payload);
	return unavailableBoard(answer.state === "unavailable" ? answer.reason : answer.state);
}
/** The source the page reads: the owner's board through `x/-/ops`. */
var opsSource = { async board(signal) {
	return boardFromAnswer(await ownerRead("ops", signal));
} };
//#endregion
//#region src/ops-page.ts
/** The Ops web view, read-only: Agents by squad, Squads, Timers and Ops settings, one section at a
* time from the module menu. Rows are the shared list rows; every state is a mark and a word. */
/** The board's own reload cadence (`tmt ops ui`). */
var REFRESH_MS = 5e3;
var SECTIONS$1 = [
	{
		id: "agents",
		label: "Agents"
	},
	{
		id: "squads",
		label: "Squads"
	},
	{
		id: "schedules",
		label: "Timers"
	},
	{
		id: "settings",
		label: "Ops settings"
	}
];
function row$1(mark, tone, title, detail, meta = "") {
	const item = document.createElement("li");
	item.className = "tmt-ui-list-row";
	item.dataset.leading = "";
	const sign = document.createElement("div");
	sign.className = "tmt-ui-list-leading";
	sign.dataset.tone = tone;
	sign.setAttribute("aria-hidden", "true");
	sign.textContent = mark;
	const name = document.createElement("div");
	name.className = "tmt-ui-list-title";
	name.textContent = title;
	const info = document.createElement("div");
	info.className = "tmt-ui-list-meta";
	const more = document.createElement("div");
	more.className = "tmt-ui-list-updated";
	more.textContent = detail;
	more.title = detail;
	const state = document.createElement("div");
	state.className = "tmt-ui-list-state";
	state.textContent = meta;
	info.append(more, state);
	item.append(sign, name, info);
	return item;
}
function list(rows) {
	const node = document.createElement("ul");
	node.className = "tmt-ui-list";
	node.append(...rows);
	return node;
}
/** A sentence in place of a list: why there is nothing to show, with any command as code. */
function sentence(text) {
	const node = document.createElement("p");
	node.className = "ops-empty";
	notAvailable(node, text);
	return node;
}
function heading$1(text) {
	const node = document.createElement("h2");
	node.className = browserUiClasses.sectionHeading;
	node.textContent = text;
	return node;
}
/** The section's content; agents group under their squad, the lead first. */
function opsSection(board, section) {
	if (section === "agents") return board.squads.flatMap((squad) => {
		const agents = board.agents.filter((agent) => agent.squad === squad.name).sort((a, b) => Number(b.role === "lead") - Number(a.role === "lead"));
		if (!agents.length) return [];
		return [heading$1(squad.lead ? `${squad.name} · lead ${squad.lead}` : squad.name), list(agents.map((agent) => {
			const state = AGENT_STATES[agent.state];
			const detail = [
				agent.stateText ?? state.word,
				agent.pending ? `Waiting on you: ${agent.pending}` : "",
				agent.waitingOnYou > 0 ? `${agent.waitingOnYou} waiting` : "",
				agent.task ?? "",
				agent.next ? `next ${agent.next}` : ""
			].filter(Boolean);
			return row$1(state.mark, state.tone, agent.name, detail.join(" · "), agent.model ?? "");
		}))];
	});
	if (section === "squads") return [list(board.squads.map((squad) => {
		const count = board.agents.filter((agent) => agent.squad === squad.name).length;
		return row$1("●", "neutral", squad.name, `${squad.lead ? `lead ${squad.lead} · ` : ""}${count} agent${count === 1 ? "" : "s"}` + (squad.idleAlertMinutes ? ` · idle alert ${squad.idleAlertMinutes}m` : ""));
	}))];
	if (section === "schedules") {
		if (board.schedules === null) return [sentence("Your machine could not read the timers. Run `tmt ops ui` to see them.")];
		if (!board.schedules.length) return [sentence("No timers are set.")];
		return [list(board.schedules.map((schedule) => row$1("◷", "neutral", schedule.what || schedule.id, [
			schedule.squad,
			schedule.when,
			schedule.owner
		].filter(Boolean).join(" · "), schedule.id)))];
	}
	return [sentence(notInBrowser("The Ops settings page", "tmt ops ui"))];
}
function opsPage(shell, source = opsSource) {
	const main = document.getElementById("ops-section");
	const title = document.querySelector(".tmt-ui-title");
	const note = document.getElementById("ops-note");
	if (!main || !note) return;
	const href = (id) => `${location.pathname}#${id}`;
	const current = () => SECTIONS$1.find((section) => `#${section.id}` === location.hash)?.id ?? "agents";
	let board;
	const paint = () => {
		const section = current();
		shell.setMenu({
			title: "Ops",
			groups: [{ items: SECTIONS$1.filter((item) => item.id !== "settings").map((item) => ({
				id: item.id,
				label: item.label,
				href: href(item.id),
				current: item.id === section
			})) }, { items: [{
				id: "settings",
				label: "Ops settings",
				href: href("settings"),
				current: section === "settings"
			}] }]
		});
		if (title) title.textContent = SECTIONS$1.find((item) => item.id === section).label;
		main.replaceChildren(...board ? opsSection(board, section) : [pageLoading()]);
	};
	window.addEventListener("hashchange", paint, { signal: shell.signal });
	paint();
	let last = "";
	whileVisible(REFRESH_MS, shell.signal, async () => {
		const next = await source.board(shell.signal);
		if (shell.signal.aborted) return true;
		const seen = JSON.stringify(next);
		if (seen !== last) {
			last = seen;
			if (next.unavailable) {
				board = void 0;
				main.replaceChildren();
				notAvailable(note, next.unavailable);
			} else {
				board = next;
				if (next.truncated) notAvailable(note, "The board is larger than the browser shows. `tmt ops ui` shows all of it.");
				else {
					note.textContent = "";
					note.hidden = true;
				}
				paint();
			}
		}
		return next.unavailable === void 0;
	});
}
//#endregion
//#region ../../../../design/browser-ui/src/theme.ts
/**
* The per-browser colour theme (#3937). The tokens answer to `data-theme` on the root: `light` and
* `dark` pin a theme, and no attribute follows the device (`prefers-color-scheme`). The choice is
* stored in this browser only; storage that is blocked or full never breaks the page, which then
* follows the device.
*/
var THEMES = [
	"system",
	"light",
	"dark"
];
var themeStorageKey = "tmt-ui-theme";
function browserStore() {
	try {
		return globalThis.localStorage;
	} catch {
		return;
	}
}
/** The stored choice; `system` when none is stored, storage is blocked or the value is unknown. */
function storedTheme(store = browserStore()) {
	try {
		const value = store?.getItem(themeStorageKey);
		return THEMES.find((theme) => theme === value) ?? "system";
	} catch {
		return "system";
	}
}
/** Set the root's theme for a choice: `system` leaves the tokens to the device. */
function applyTheme(choice, root = document.documentElement) {
	if (choice === "system") root.removeAttribute("data-theme");
	else root.setAttribute("data-theme", choice);
}
/** Apply the stored choice; call once when the app starts, before the first paint of a page. */
function applyStoredTheme(root, store) {
	applyTheme(storedTheme(store), root);
}
/** Store a choice for this browser and apply it now. `system` clears the stored choice. */
function setStoredTheme(choice, root, store = browserStore()) {
	try {
		if (choice === "system") store?.removeItem(themeStorageKey);
		else store?.setItem(themeStorageKey, choice);
	} catch {}
	applyTheme(choice, root);
}
//#endregion
//#region src/settings-source.ts
/**
* What the TMT settings page (`/settings`) reads: this machine's identities, installed modules,
* host drivers, agent hooks and a few general settings. The page depends only on `SettingsSource`.
* The source below is a MOCK until Core exposes a read API; the dev preview swaps in reads of the
* read-only CLI JSON (`tmt ls`, `tmt extension ls`, `tmt driver ls`, `tmt setup --status`,
* `tmt config show`), which hold the same facts on the terminal side.
*/
/** The source the page reads by default. */
var settingsSource = { async read() {
	const none = {
		rows: [],
		unavailable: notInBrowser("TMT settings", "tmt config show")
	};
	return {
		general: none,
		identities: none,
		modules: none,
		drivers: none,
		hooks: none,
		sample: false
	};
} };
//#endregion
//#region src/tmt-settings-page.ts
/**
* TMT's own settings (`/settings`), one section at a time from the module menu: General (theme,
* then read-only values), Identities, Modules and Drivers & hooks. Everything but the theme is
* read-only; rows are the shared list rows and every state is a mark and a word.
*/
var SECTIONS = [
	{
		id: "general",
		label: "General"
	},
	{
		id: "identities",
		label: "Identities"
	},
	{
		id: "modules",
		label: "Modules"
	},
	{
		id: "drivers",
		label: "Drivers & hooks"
	}
];
var THEME_LABELS = {
	system: "System",
	light: "Light",
	dark: "Dark"
};
/** Each described module's rail label and first settings page, by extension name. */
function moduleLinks(answer) {
	const links = /* @__PURE__ */ new Map();
	for (const module of answer?.modules ?? []) links.set(module.extension, {
		label: module.rail.label,
		href: module.settings[0]?.path
	});
	return links;
}
var capitalize = (name) => name.charAt(0).toUpperCase() + name.slice(1);
/** The section's rows; identities show live agents first, then by name. */
function settingsRows(section, settings, links = /* @__PURE__ */ new Map()) {
	if (section === "general") return settings.general.rows.map((row) => ({
		mark: "",
		title: row.name,
		detail: row.description,
		state: row.value
	}));
	if (section === "identities") return [...settings.identities.rows].sort((a, b) => Number(b.online) - Number(a.online) || a.name.localeCompare(b.name)).map((row) => ({
		mark: row.online ? "●" : "○",
		title: row.name,
		detail: row.provider ? `${row.lifetime} · ${row.provider}` : row.lifetime,
		state: row.online ? "active" : "offline"
	}));
	if (section === "modules") return settings.modules.rows.map((row) => {
		const link = links.get(row.name);
		return {
			mark: row.installed ? "●" : "○",
			title: link?.label ?? capitalize(row.name),
			...row.installed && link?.href ? { href: link.href } : {},
			detail: row.installed ? [row.version, row.channel].filter(Boolean).join(" · ") : "Not installed",
			state: row.installed ? "installed" : ""
		};
	});
	return [...settings.drivers.rows.map((row) => ({
		mark: row.state === "ok" ? "●" : "◆",
		waiting: row.state !== "ok",
		title: row.name,
		detail: `Host driver ${row.version}`,
		state: row.state === "ok" ? "approved" : row.state
	})), ...settings.hooks.rows.map((row) => ({
		mark: "●",
		title: row.provider,
		detail: `Agent hooks · usage collection ${row.usage ? "on" : "off"}`,
		state: "set up"
	}))];
}
/** Why a section has nothing to list: the source could not read it, or there is none. */
function emptyText(section, settings) {
	const unavailable = (section === "drivers" ? [settings.drivers, settings.hooks] : [settings[section]]).map((part) => part.unavailable).find(Boolean);
	if (unavailable) return unavailable;
	return {
		general: "No settings to show.",
		identities: "No identities yet.",
		modules: "No modules.",
		drivers: "No drivers or agent hooks are set up."
	}[section];
}
function row(item) {
	const node = document.createElement("li");
	node.className = "tmt-ui-list-row";
	node.dataset.leading = "";
	const sign = document.createElement("div");
	sign.className = "tmt-ui-list-leading";
	sign.setAttribute("aria-hidden", "true");
	sign.textContent = item.mark;
	if (item.waiting) sign.dataset.tone = "waiting";
	const name = document.createElement("div");
	name.className = "tmt-ui-list-title";
	const label = document.createElement("span");
	label.className = "tmt-ui-list-label";
	if (item.href) {
		const link = document.createElement("a");
		link.href = item.href;
		link.textContent = item.title;
		label.append(link);
	} else label.textContent = item.title;
	name.append(label);
	if (item.detail) {
		const detail = document.createElement("span");
		detail.className = "tmt-ui-list-description";
		detail.textContent = item.detail;
		name.append(detail);
	}
	const info = document.createElement("div");
	info.className = "tmt-ui-list-meta";
	const state = document.createElement("div");
	state.className = "tmt-ui-list-state";
	state.textContent = item.state;
	info.append(state);
	node.append(sign, name, info);
	return node;
}
function heading(text) {
	const node = document.createElement("h2");
	node.className = browserUiClasses.sectionHeading;
	node.textContent = text;
	return node;
}
/** The theme choice: one tab per theme; picking one stores it for this browser and applies it. */
function themeChoice() {
	const group = document.createElement("div");
	group.className = "tmt-ui-tab-list";
	group.setAttribute("role", "tablist");
	group.setAttribute("aria-label", "Theme");
	let current = storedTheme();
	const buttons = THEMES.map((theme) => {
		const button = document.createElement("button");
		button.type = "button";
		button.className = "tmt-ui-tab";
		button.setAttribute("role", "tab");
		button.dataset.choice = theme;
		const label = document.createElement("span");
		label.className = "tmt-ui-tab-label";
		label.textContent = THEME_LABELS[theme];
		button.append(label);
		return button;
	});
	const show = (theme) => {
		current = theme;
		for (const button of buttons) {
			const selected = button.dataset.choice === theme;
			button.setAttribute("aria-selected", String(selected));
			button.tabIndex = selected ? 0 : -1;
		}
	};
	const choose = (theme) => {
		setStoredTheme(theme);
		show(theme);
	};
	for (const button of buttons) {
		button.addEventListener("click", () => choose(button.dataset.choice));
		button.addEventListener("keydown", (event) => {
			const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
			if (!step) return;
			event.preventDefault();
			const next = THEMES[(THEMES.indexOf(current) + step + THEMES.length) % THEMES.length];
			choose(next);
			buttons[THEMES.indexOf(next)].focus();
		});
	}
	show(current);
	group.append(...buttons);
	return group;
}
/** The section's content. */
function settingsSection(section, settings, links) {
	const rows = settingsRows(section, settings, links);
	const body = [];
	if (rows.length) {
		const list = document.createElement("ul");
		list.className = "tmt-ui-list";
		list.append(...rows.map(row));
		body.push(list);
	} else {
		const none = document.createElement("p");
		none.className = "home-note";
		notAvailable(none, emptyText(section, settings));
		body.push(none);
	}
	if (section !== "general") return body;
	const help = document.createElement("p");
	help.className = "home-note";
	help.textContent = "System follows this device. Your choice is kept in this browser only.";
	return [
		heading("Theme"),
		themeChoice(),
		help,
		heading("Settings"),
		...body
	];
}
/** The page in its route; everything it starts ends on the route's `shell.signal`. */
function tmtSettingsPage(shell, source = settingsSource) {
	const main = document.getElementById("tmt-settings-section");
	const note = document.getElementById("tmt-settings-note");
	if (!main || !note) return;
	const title = document.querySelector(".tmt-ui-title");
	const href = (id) => `${location.pathname}#${id}`;
	const current = () => SECTIONS.find((section) => `#${section.id}` === location.hash)?.id ?? "general";
	let settings;
	let links = /* @__PURE__ */ new Map();
	const live = () => !shell.signal.aborted;
	const paint = () => {
		if (!live()) return;
		const section = current();
		shell.setMenu({
			title: "TMT settings",
			groups: [{ items: SECTIONS.map((item) => ({
				id: item.id,
				label: item.label,
				href: href(item.id),
				current: item.id === section
			})) }]
		});
		if (title) title.textContent = SECTIONS.find((item) => item.id === section).label;
		main.replaceChildren(...settings ? settingsSection(section, settings, links) : [pageLoading()]);
	};
	window.addEventListener("hashchange", paint, { signal: shell.signal });
	shell.described((answer) => {
		links = moduleLinks(answer);
		paint();
	});
	paint();
	source.read().then((next) => {
		if (!live()) return;
		settings = next;
		note.textContent = next.sample ? "Sample data: Core has no web read of this machine yet. `tmt ls`, `tmt extension ls` and `tmt driver ls` show the real values." : "";
		note.hidden = !next.sample;
		paint();
	}, () => {
		if (!live()) return;
		main.replaceChildren();
		note.textContent = "Settings could not be read.";
		note.hidden = false;
	});
}
//#endregion
//#region src/shell-app.ts
/**
* The one TMT web app the door serves at every TMT route (#3937): one document, one shell, one
* router. Remote's own pages are route modules here; Colab is loaded lazily from its own mount.
*/
/**
* Put a page's markup into the slot: its header into the shell bar, its content into the main
* area, its title and page name onto the document. The page files stay the one source of each
* page's markup; the door no longer serves them as documents.
*/
async function adopt(slot, html) {
	const page = new DOMParser().parseFromString(html, "text/html");
	const release = await pageStyles(page);
	const bar = page.querySelector(".tmt-ui-shell-bar");
	const main = page.querySelector(".tmt-ui-shell-main");
	slot.bar.replaceChildren(...bar ? [...bar.childNodes].map((node) => document.adoptNode(node)) : []);
	slot.main.replaceChildren(...main ? [...main.childNodes].map((node) => document.adoptNode(node)) : []);
	document.documentElement.dataset.tmtPage = page.documentElement.dataset.tmtPage ?? "";
	slot.setTitle(page.title.replace(/ · TMT$/, ""));
	return release;
}
/**
* The door's own stylesheets a page names beyond the shell's `/sdk/pages.css` (Remote settings has
* `/sdk/settings.css`): linked once the route mounts, removed when it unmounts. Resolves when each
* has loaded or failed, so the markup never shows unstyled.
*/
async function pageStyles(page) {
	const added = [];
	const loading = [];
	for (const source of page.querySelectorAll("link[rel=\"stylesheet\"]")) {
		const href = source.getAttribute("href");
		if (!href?.startsWith("/sdk/") || href === "/sdk/pages.css" || [...document.head.querySelectorAll("link")].some((link) => link.getAttribute("href") === href)) continue;
		const link = document.createElement("link");
		link.rel = "stylesheet";
		link.href = href;
		loading.push(new Promise((resolve) => {
			link.onload = link.onerror = () => resolve();
		}));
		document.head.append(link);
		added.push(link);
	}
	await Promise.all(loading);
	return () => added.forEach((link) => link.remove());
}
/** A Remote page as a route module: its markup, then its page code, released on unmount. */
function remotePage(html, run) {
	return { async mount(slot) {
		const unstyle = await adopt(slot, html);
		const lifetime = new AbortController();
		hostRemote(slot, lifetime.signal);
		const end = () => {
			lifetime.abort();
			unstyle();
			hostRemote(void 0);
		};
		let release;
		try {
			release = await run(slot);
		} catch (error) {
			end();
			throw error;
		}
		return () => {
			release?.();
			end();
		};
	} };
}
/** The script an older Colab, which names no `entry`, is loaded from under its own mount. */
var COLAB_ENTRY = "assets/colab-module.js";
/** Colab as the composed answer describes it: its own link, and its entry only if confined. */
function colabLoad(answer) {
	const colab = answer?.modules.find((module) => module.extension === "colab");
	if (!colab) return void 0;
	return moduleLoad(colab.rail.path, colab.entry, location.href, COLAB_ENTRY);
}
function message(slot, title, text, link) {
	const main = document.createElement("section");
	main.className = "tmt-page";
	const heading = document.createElement("h2");
	heading.textContent = title;
	const body = document.createElement("p");
	body.textContent = text;
	main.append(heading, body);
	if (link) {
		const open = document.createElement("p");
		const anchor = document.createElement("a");
		anchor.href = link.href;
		anchor.textContent = link.label;
		open.append(anchor);
		main.append(open);
	}
	slot.main.replaceChildren(main);
	slot.setTitle(title);
}
var routes = [
	{
		id: "home",
		paths: ["/"],
		rail: "home",
		load: async () => remotePage(home_default, () => homePage(remoteShell(), (href) => appHref(routes, href)))
	},
	{
		id: "remote",
		paths: ["/remote", "/remote/"],
		rail: "remote",
		quiet: true,
		load: async () => remotePage(landing_default, async () => {
			const { landingPage } = await import("./remote-v1.js");
			await landingPage(remoteShell().signal);
		})
	},
	{
		id: "remote-settings",
		paths: ["/remote/settings"],
		rail: "remote",
		load: async () => remotePage(settings_default, async () => {
			const { settingsPage } = await import(
				/* @vite-ignore */
				settingsEntry
);
			await settingsPage(remoteShell());
		})
	},
	{
		id: "ops",
		paths: ["/ops", "/ops/"],
		rail: "ops",
		load: async () => remotePage(ops_default, () => opsPage(remoteShell()))
	},
	{
		id: "digest",
		paths: ["/digest", "/digest/"],
		rail: "digest",
		load: async () => remotePage(digest_default, () => digestPage(void 0, remoteShell().signal))
	},
	{
		id: "settings",
		paths: ["/settings", "/settings/"],
		rail: "settings",
		load: async () => remotePage(tmt_settings_default, () => tmtSettingsPage(remoteShell()))
	},
	{
		id: "colab",
		paths: ["/colab", "/colab/"],
		rail: "colab",
		async load(_remembered, settled) {
			let answer = await settled();
			if (!answer && await reopened()) answer = await started?.redescribe();
			const load = colabLoad(answer);
			const entry = load?.entry;
			if (!entry) return { mount: (slot) => load ? message(slot, "Colab", "Colab could not be opened here.", load.link ? {
				href: load.link,
				label: "Open Colab"
			} : void 0) : answer ? message(slot, "Colab", "Colab is not running on your machine. Run tmt colab serve there, then reload.") : message(slot, "Colab", "Pair this browser with Remote to open Colab. On the machine that runs Remote, run tmt remote pair.", {
				href: "/remote/",
				label: "Pair this browser"
			}) };
			try {
				return await import(
					/* @vite-ignore */
					entry
);
			} catch {
				return { mount: (slot) => message(slot, "Colab", "Colab could not start here. It may have stopped on your machine. Reload to try again, or open Colab on its own page.", load.link ? {
					href: load.link,
					label: "Open Colab"
				} : void 0) };
			}
		}
	}
];
/** Whether this browser's pairing reopened a door session. */
async function reopened() {
	try {
		const { reopenSession } = await import("./remote-v1.js");
		await reopenSession();
		return true;
	} catch {
		return false;
	}
}
/** Remote's settings page code, served beside the SDK. */
var settingsEntry = "/sdk/settings-v1.js";
var started;
/** Start the app once in the served shell document. */
function shellApp() {
	const root = document.querySelector(".tmt-ui-shell");
	if (!root || started || root.dataset.tmtRail) return started;
	applyStoredTheme();
	if (document.documentElement.dataset.tmtPage === "pair") {
		root.dataset.tmtRail = "static";
		const place = {
			origin: location.origin,
			href: location.href.split(/[?#]/)[0]
		};
		const remote = routes.find((route) => route.id === "remote");
		mountShell(root, { rail: appRail(rememberedShellAnswer(), routes, place, remote) });
		return;
	}
	started = startShellHost(root, routes, { failed: (main) => main.replaceChildren(Object.assign(document.createElement("p"), {
		className: "page",
		textContent: "This page could not load. Reload to try again."
	})) });
	return started;
}
//#endregion
//#region src/budget.ts
var budget_exports = /* @__PURE__ */ __exportAll({
	ACTIVE_SECONDS_PER_DAY: () => ACTIVE_SECONDS_PER_DAY,
	BUDGET_EXHAUSTED: () => BUDGET_EXHAUSTED,
	BudgetModel: () => BudgetModel,
	BudgetModelError: () => BudgetModelError,
	MAX_MEMBERS: () => MAX_MEMBERS,
	READS_PER_DAY: () => READS_PER_DAY,
	READ_LOOKUPS: () => 2,
	REFUSE_PERCENT: () => 90,
	WARN_PERCENT: () => 70,
	WRITES_PER_DAY: () => WRITES_PER_DAY,
	WRITE_LOOKUPS: () => 4,
	classifyProviderExhausted: () => classifyProviderExhausted,
	decide: () => decide,
	exhaustedIsCurrent: () => exhaustedIsCurrent,
	newUsage: () => newUsage,
	pacificDay: () => pacificDay,
	parsePublishedLimits: () => parsePublishedLimits,
	publishedLimitRows: () => publishedLimitRows,
	recordedUsage: () => recordedUsage,
	usageAt: () => usageAt
});
/**
* Free-plan (Spark) Firestore budget guard, the browser twin of the Rust `firestore_budget`
* module (#2180). Pure arithmetic: no I/O and no clock (callers pass `nowMs`). Quotas are
* project-wide, every client sees only its own traffic and Rules cannot read quotas, so the
* guard works from what a client really has: the member count from the signed membership, the
* page's parameters, its own persisted counter for today and these constants. It does not read
* Google's counter, and the counter a client keeps is per browser and device, so other
* devices' appends are invisible to it: `decide` is a local estimate and the provider's own
* `resource-exhausted` answer is the backstop. Both implementations are tested against the
* same independent vectors, so a change to either one fails until both agree; the dated
* numbers and the derivation are in the Remote reference guide's free-plan table.
*/
/** Spark (no-cost) Firestore document reads per day. */
var READS_PER_DAY = 5e4;
/** Spark (no-cost) Firestore document writes per day. */
var WRITES_PER_DAY = 2e4;
/** Planning assumption for the minimum flush interval: this many active seconds per day. */
var ACTIVE_SECONDS_PER_DAY = 7200;
/** Largest member count the model accepts. */
var MAX_MEMBERS = 1e4;
/** The error code of a refusal made before any effect. */
var BUDGET_EXHAUSTED = "REMOTE_BUDGET_EXHAUSTED";
/**
* `out_of_range`: no members, no writers, more writers than members, or too many members.
* `free_plan_cannot_host`: the modeled share is too small for the guard to allow even one
* append. A page like that is refused when the model is built, never as a pause that would end
* at the next reset and begin again.
*/
var BudgetModelError = class extends Error {
	code;
	constructor(code) {
		super(code === "out_of_range" ? "The page size is out of range." : "The free plan cannot host a page this size.");
		this.code = code;
		this.name = "BudgetModelError";
	}
};
/** One shared page: `members` have it open (every one a listener), `writers` of them append. */
var BudgetModel = class BudgetModel {
	members;
	writers;
	constructor(members, writers) {
		this.members = members;
		this.writers = writers;
	}
	static create(members, writers) {
		const whole = (value) => Number.isSafeInteger(value);
		if (!(whole(members) && whole(writers) && members >= 1 && members <= 1e4 && writers >= 1 && writers <= members)) throw new BudgetModelError("out_of_range");
		const model = new BudgetModel(members, writers);
		if (model.refuseAt() === 0) throw new BudgetModelError("free_plan_cannot_host");
		return model;
	}
	/** Reads one append costs the project, pessimistically: the writer's lookups, then each other listener's. */
	readsPerAppend() {
		return 4 + (this.members - 1) * 3;
	}
	/** Appends per day the free plan allows this page if it were the project's only traffic. */
	dailyAppendLimit() {
		return Math.min(Math.floor(READS_PER_DAY / this.readsPerAppend()), WRITES_PER_DAY);
	}
	/** One client's modeled share of the page's daily allowance. */
	share() {
		return Math.floor(this.dailyAppendLimit() / this.writers);
	}
	/** Appends after which the guard warns, and the count at which it refuses. */
	warnAt() {
		return Math.floor(this.share() * 70 / 100);
	}
	refuseAt() {
		return Math.floor(this.share() * 90 / 100);
	}
	/** Shortest average interval between flushed updates, in milliseconds, that keeps the planning assumption. */
	minFlushIntervalMs() {
		const appends = Math.max(Math.floor(this.dailyAppendLimit() * 70 / 100), 1);
		return Math.ceil(ACTIVE_SECONDS_PER_DAY * 1e3 / appends);
	}
	/** What the guard says about the next append, given how many this client sent today. */
	assess(appendsToday) {
		if (appendsToday >= this.refuseAt()) return "refuse";
		if (appendsToday >= this.warnAt()) return "warn";
		return "ok";
	}
};
var DAY_MS = 864e5;
var HOUR_MS = 36e5;
var floorDiv = (value, divisor) => Math.floor(value / divisor);
/** Days since 1970-01-01 of a proleptic Gregorian date. */
function daysFromCivil(year, month, day) {
	const y = month <= 2 ? year - 1 : year;
	const era = floorDiv(y, 400);
	const yoe = y - era * 400;
	const doy = floorDiv(153 * ((month + 9) % 12) + 2, 5) + day - 1;
	const doe = yoe * 365 + floorDiv(yoe, 4) - floorDiv(yoe, 100) + doy;
	return era * 146097 + doe - 719468;
}
function civilYear(days) {
	const z = days + 719468;
	const era = floorDiv(z, 146097);
	const doe = z - era * 146097;
	const yoe = floorDiv(doe - floorDiv(doe, 1460) + floorDiv(doe, 36524) - floorDiv(doe, 146096), 365);
	const mp = floorDiv(5 * (doe - (365 * yoe + floorDiv(yoe, 4) - floorDiv(yoe, 100))) + 2, 153);
	const month = mp < 10 ? mp + 3 : mp - 9;
	return yoe + era * 400 + (month <= 2 ? 1 : 0);
}
/** The n-th Sunday (1-based) of `month` in `year`, as a day number. */
function nthSunday(year, month, n) {
	const first = daysFromCivil(year, month, 1);
	return first + (7 - ((first + 4) % 7 + 7) % 7) % 7 + (n - 1) * 7;
}
/** United States rule since 2007: daylight time from 02:00 local on the second Sunday of March to 02:00 local on the first Sunday of November. */
function isDstLocal(localMs) {
	const year = civilYear(floorDiv(localMs, DAY_MS));
	const start = nthSunday(year, 3, 2) * DAY_MS + 2 * HOUR_MS;
	const end = nthSunday(year, 11, 1) * DAY_MS + 2 * HOUR_MS;
	return localMs >= start && localMs < end;
}
/** The Pacific UTC offset at a UTC instant: -8 h, or -7 h in daylight time. */
function offsetMs(utcMs) {
	const year = civilYear(floorDiv(utcMs - 8 * HOUR_MS, DAY_MS));
	const start = nthSunday(year, 3, 2) * DAY_MS + 10 * HOUR_MS;
	const end = nthSunday(year, 11, 1) * DAY_MS + 9 * HOUR_MS;
	return utcMs >= start && utcMs < end ? -7 * HOUR_MS : -8 * HOUR_MS;
}
function pacificDay(nowMs) {
	const day = floorDiv(nowMs + offsetMs(nowMs), DAY_MS);
	const nextMidnightLocal = (day + 1) * DAY_MS;
	return {
		day,
		resetAtMs: nextMidnightLocal - (isDstLocal(nextMidnightLocal) ? -7 * HOUR_MS : -8 * HOUR_MS)
	};
}
function newUsage(nowMs) {
	return {
		day: pacificDay(nowMs).day,
		appends: 0
	};
}
/** The same usage if still the same Pacific day, else a fresh day. */
function usageAt(usage, nowMs) {
	return pacificDay(nowMs).day === usage.day ? usage : newUsage(nowMs);
}
/** Count one append sent. */
function recordedUsage(usage, nowMs) {
	const today = usageAt(usage, nowMs);
	return {
		...today,
		appends: Math.min(today.appends + 1, Number.MAX_SAFE_INTEGER)
	};
}
/** Judge the next append. Rolls the usage over to today first. */
function decide(model, usage, nowMs) {
	const today = usageAt(usage, nowMs);
	const verdict = model.assess(today.appends);
	if (verdict === "ok") return { decision: "allow" };
	if (verdict === "warn") return { decision: "warn" };
	const { resetAtMs } = pacificDay(nowMs);
	return {
		decision: "refuse",
		resetAtMs,
		retryAfterMs: Math.max(resetAtMs - nowMs, 0)
	};
}
function classifyProviderExhausted(attempt, nowMs) {
	const { day, resetAtMs } = pacificDay(nowMs);
	const evidence = {
		day,
		resetAtMs
	};
	if (attempt.mutation) return {
		outcome: { outcome: "unknown" },
		evidence
	};
	return {
		outcome: {
			outcome: "refused",
			resetAtMs,
			retryAfterMs: Math.max(resetAtMs - nowMs, 0)
		},
		evidence
	};
}
/** Still a statement about today's quota. */
function exhaustedIsCurrent(evidence, nowMs) {
	return nowMs < evidence.resetAtMs;
}
var publishedLimitRows = [
	[
		"readsPerDay",
		"Document reads",
		"day",
		false
	],
	[
		"writesPerDay",
		"Document writes",
		"day",
		false
	],
	[
		"deletesPerDay",
		"Document deletes",
		"day",
		false
	],
	[
		"storedBytes",
		"Stored data",
		"",
		true
	],
	[
		"egressBytesPerMonth",
		"Data sent out",
		"month",
		true
	],
	[
		"databases",
		"Free databases per project",
		"",
		false
	],
	[
		"compositeIndexes",
		"Composite indexes",
		"",
		false
	],
	[
		"singleFieldIndexConfigs",
		"Single-field index configs",
		"",
		false
	],
	[
		"documentBytes",
		"Document size",
		"",
		true
	],
	[
		"rulesLookupsPerRequest",
		"Rules document lookups per request",
		"",
		false
	]
];
function parsePublishedLimits(value) {
	const object = (v) => typeof v === "object" && v !== null && !Array.isArray(v);
	const keys = (v, expected) => Object.keys(v).length === expected.length && expected.every((key) => Object.hasOwn(v, key));
	if (!object(value) || !keys(value, [
		"plan",
		"readOn",
		"resetsAt",
		"limits",
		"guard"
	]) || value.plan !== "no-cost" || value.resetsAt !== "pacific-midnight" || typeof value.readOn !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(value.readOn) || !object(value.limits) || !keys(value.limits, publishedLimitRows.map((row) => row[0])) || !Object.values(value.limits).every((n) => typeof n === "number" && Number.isSafeInteger(n) && n > 0) || !object(value.guard) || !keys(value.guard, ["warnPercent", "refusePercent"]) || !Number.isSafeInteger(value.guard.warnPercent) || !Number.isSafeInteger(value.guard.refusePercent) || !(value.guard.warnPercent > 0 && value.guard.warnPercent < value.guard.refusePercent && value.guard.refusePercent <= 100)) throw new Error("Invalid Firestore budget response.");
	return value;
}
var unavailable$2 = () => ({ code: "REMOTE_STATE_UNAVAILABLE" });
function syncCallLive(call) {
	if (call.signal.aborted || !Number.isFinite(call.deadlineAtMs) || Date.now() >= call.deadlineAtMs) throw unavailable$2();
}
/** Bound every injected await even if a fake or provider ignores its AbortSignal. */
async function syncWait(call, action) {
	syncCallLive(call);
	let timer;
	let abort;
	const end = new Promise((_, reject) => {
		abort = () => reject(unavailable$2());
		call.signal.addEventListener("abort", abort, { once: true });
		timer = setTimeout(abort, Math.max(0, call.deadlineAtMs - Date.now()));
	});
	try {
		return await Promise.race([action(), end]);
	} finally {
		clearTimeout(timer);
		if (abort) call.signal.removeEventListener("abort", abort);
	}
}
/** Keep caller guard failures in the frozen refusal vocabulary; never expose its exception text. */
async function admitSync(guard, cost, call) {
	try {
		await syncWait(call, () => guard.admit(Object.freeze(cost)));
	} catch (error) {
		throw syncGuardFailure(error);
	}
}
/** Preserve only the frozen guard refusal, never caller exception text. */
function syncGuardFailure(error) {
	if (typeof error === "object" && error !== null && "code" in error && error.code === "REMOTE_SCOPE_DENIED") return { code: "REMOTE_SCOPE_DENIED" };
	if (typeof error === "object" && error !== null && "code" in error && error.code === "REMOTE_BUDGET_EXHAUSTED" && "resetAtMs" in error && typeof error.resetAtMs === "number" && Number.isFinite(error.resetAtMs) && error.resetAtMs >= 0 && "retryAfterMs" in error && typeof error.retryAfterMs === "number" && Number.isFinite(error.retryAfterMs) && error.retryAfterMs >= 0) return {
		code: "REMOTE_BUDGET_EXHAUSTED",
		resetAtMs: error.resetAtMs,
		retryAfterMs: error.retryAfterMs
	};
	return unavailable$2();
}
/** The owner fence checks its uid/project/generation after every asynchronous boundary. */
function syncHttp(context, guard, fetcher, live) {
	const base = `https://firestore.googleapis.com/v1/projects/${context.config.projectId}/databases/(default)/documents`;
	return async (operation, path, body, call, reads, mutation) => {
		const check = () => {
			live();
			syncCallLive(call);
		};
		check();
		if (body !== void 0 && new TextEncoder().encode(body).byteLength > 397312) throw { code: "REMOTE_INPUT_INVALID" };
		await admitSync(guard, {
			operation,
			reads,
			writes: operation === "commit" ? 1 : 0,
			rulesLookupsPerDocument: operation === "commit" ? 4 : 3,
			...mutation ? { mutation } : {}
		}, call);
		check();
		const credential = await syncWait(call, () => context.auth.token(call));
		check();
		if ("state" in credential) throw credential;
		if (credential.projectId !== context.config.projectId || credential.uid !== context.uid || credential.expiresAtMs <= Date.now() || !Number.isFinite(credential.expiresAtMs) || !/^[\x21-\x7e]{1,16384}$/.test(credential.token)) throw unavailable$2();
		const controller = new AbortController();
		const abort = () => controller.abort();
		call.signal.addEventListener("abort", abort, { once: true });
		const timer = setTimeout(abort, Math.max(0, call.deadlineAtMs - Date.now()));
		let sent = false;
		try {
			check();
			const url = operation === "read" ? `${base}/${path.split("/").map(encodeURIComponent).join("/")}` : `${base}${operation === "query" && path ? "/" + path.split("/").map(encodeURIComponent).join("/") : ""}:${operation === "query" ? "runQuery" : "commit"}`;
			sent = true;
			const response = await syncWait(call, () => fetcher(url, {
				method: operation === "read" ? "GET" : "POST",
				headers: {
					Authorization: `Bearer ${credential.token}`,
					"Content-Type": "application/json"
				},
				body,
				signal: controller.signal,
				redirect: "error",
				credentials: "omit",
				cache: "no-store"
			}));
			check();
			if (controller.signal.aborted) throw unavailable$2();
			const reader = response.body?.getReader();
			if (!reader) throw unavailable$2();
			const chunks = [];
			let length = 0;
			try {
				for (;;) {
					const chunk = await syncWait(call, () => reader.read());
					check();
					if (controller.signal.aborted) throw unavailable$2();
					if (chunk.done) break;
					length += chunk.value.byteLength;
					if (length > 397312) throw unavailable$2();
					chunks.push(chunk.value);
				}
			} finally {
				reader.cancel().catch(() => {});
				reader.releaseLock();
			}
			check();
			const bytes = new Uint8Array(length);
			let offset = 0;
			for (const chunk of chunks) {
				bytes.set(chunk, offset);
				offset += chunk.byteLength;
			}
			const value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
			check();
			return {
				kind: "response",
				status: response.status,
				value
			};
		} catch {
			if (operation === "commit" && sent) return { kind: "unknown" };
			throw unavailable$2();
		} finally {
			clearTimeout(timer);
			call.signal.removeEventListener("abort", abort);
		}
	};
}
//#endregion
//#region src/firebase-sync-query.ts
/** Finite stream/discovery REST pages. Only callers own logical delivery positions. */
var unavailable$1 = () => ({ code: "REMOTE_STATE_UNAVAILABLE" });
var invalid$1 = () => ({ code: "REMOTE_INPUT_INVALID" });
function createSyncQuery(context, http, resource, document, status, live) {
	const base = `projects/${context.config.projectId}/databases/(default)/documents/`;
	let continuationKey;
	const key = () => continuationKey ??= crypto.subtle.generateKey({
		name: "HMAC",
		hash: "SHA-256"
	}, false, ["sign", "verify"]);
	const encodeContinuation = async (binding, last, deadline) => {
		const bytes = new TextEncoder().encode(JSON.stringify({
			binding,
			last,
			deadline
		}));
		const signature = new Uint8Array(await crypto.subtle.sign("HMAC", await key(), bytes));
		const token = `${base64url(bytes)}.${base64url(signature)}`;
		if (token.length > 1024) throw unavailable$1();
		return token;
	};
	const decodeContinuation = async (token, binding) => {
		try {
			const parts = token.split(".");
			if (parts.length !== 2) throw invalid$1();
			const size = Math.floor(parts[0].length * 6 / 8);
			const bytes = base64urlBytes(parts[0], size);
			if (!await crypto.subtle.verify("HMAC", await key(), new Uint8Array(base64urlBytes(parts[1], 32)), new Uint8Array(bytes))) throw invalid$1();
			const row = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
			if (row.binding !== binding || typeof row.deadline !== "number" || Date.now() >= row.deadline || typeof row.last !== "string") throw invalid$1();
			live();
			return row.last;
		} catch {
			throw invalid$1();
		}
	};
	return async (request, call) => {
		const q = structuredClone(request);
		const ordinary = q.shape === "stream";
		if (ordinary) validateSyncQuery(context, q.query);
		else validateSyncDiscovery(context, q.query);
		const r = resource(q.query.resource);
		const mapping = r.sync.query;
		const limit = Math.max(1, Math.min(256, Math.floor(397312 / (r.maxObjectBytes + 4096))));
		const filters = Object.entries(q.query.scope).map(([fieldPath, value]) => ({ fieldFilter: {
			field: { fieldPath },
			op: "EQUAL",
			value
		} }));
		if (ordinary) {
			if (q.query.cursor) filters.push({ fieldFilter: {
				field: { fieldPath: mapping.seqField },
				op: "GREATER_THAN",
				value: { integerValue: String(q.query.cursor.seq) }
			} });
		} else if (mapping.discovery.seqEquals !== null) filters.push({ fieldFilter: {
			field: { fieldPath: mapping.seqField },
			op: "EQUAL",
			value: { integerValue: mapping.discovery.seqEquals }
		} });
		const segments = `x/${context.extension}/${r.path}`.split("/");
		const collectionId = segments.pop();
		const parent = segments.join("/");
		const structuredQuery = {
			from: [{ collectionId }],
			where: { compositeFilter: {
				op: "AND",
				filters
			} },
			orderBy: [{
				field: { fieldPath: ordinary ? mapping.seqField : "__name__" },
				direction: "ASCENDING"
			}],
			limit
		};
		const bound = call;
		const binding = JSON.stringify([
			context.config.projectId,
			context.declarationDigest,
			context.space,
			q.query.resource,
			q.shape === "discovery" ? q.query.shape : "",
			Object.entries(q.query.scope).sort(([a], [b]) => a < b ? -1 : 1)
		]);
		const queryBody = structuredQuery;
		if (q.shape === "discovery" && q.query.continuation) {
			const last = await syncWait(bound, () => decodeContinuation(q.query.continuation, binding));
			if (!last.startsWith(`${base}${parent}/${collectionId}/`) || !/^[A-Za-z0-9_+-]{1,256}$/.test(last.slice(`${base}${parent}/${collectionId}/`.length))) throw invalid$1();
			live();
			syncCallLive(bound);
			queryBody.startAt = {
				values: [{ referenceValue: last }],
				before: false
			};
		}
		const response = await http("query", parent, JSON.stringify({ structuredQuery: queryBody }), bound, limit);
		if (response.kind === "unknown" || response.status !== 200) {
			if (response.kind !== "unknown") status(response.status);
			throw unavailable$1();
		}
		if (!Array.isArray(response.value) || response.value.length > limit + 1) throw unavailable$1();
		const documents = [];
		for (const row of response.value) {
			if (typeof row !== "object" || row === null || Object.keys(row).some((key) => ![
				"document",
				"readTime",
				"skippedResults",
				"transaction",
				"done"
			].includes(key))) throw unavailable$1();
			if ("document" in row) documents.push(await document(row.document, r));
		}
		if (documents.length > limit) throw unavailable$1();
		for (const d of documents) {
			const fields = validateSyncFields(d.bytes);
			if (!Object.entries(q.query.scope).every(([key, value]) => JSON.stringify(fields[key]) === JSON.stringify(value))) throw unavailable$1();
		}
		live();
		syncCallLive(bound);
		if (ordinary) {
			let position = q.query.cursor?.seq ?? 0;
			for (const d of documents) {
				const fields = validateSyncFields(d.bytes);
				const sequence = Number(fields[mapping.seqField].integerValue);
				if (sequence !== position + 1) throw {
					code: "REMOTE_STATE_UNAVAILABLE",
					state: "catch-up-required"
				};
				position = sequence;
			}
			return {
				shape: "stream",
				page: {
					documents,
					...documents.length ? { cursor: {
						stream: q.query.stream,
						seq: position
					} } : {}
				}
			};
		}
		let continuation;
		if (documents.length === limit) {
			const last = `${base}${syncDocumentPath(context, documents.at(-1).target)}`;
			continuation = await syncWait(bound, () => encodeContinuation(binding, last, bound.deadlineAtMs));
			live();
			syncCallLive(bound);
		}
		return {
			shape: "discovery",
			page: {
				documents,
				...continuation ? { continuation } : {}
			}
		};
	};
}
//#endregion
//#region src/firebase-sealed-sync.ts
/** Bound sealed-document owner. REST data and push hints share identity, never delivery progress. */
var tab = {
	writes: 0,
	listeners: 0,
	streams: /* @__PURE__ */ new Set()
};
var unavailable = () => ({ code: "REMOTE_STATE_UNAVAILABLE" });
/** Identity fences retain only their typed scope verdict; private diagnostics never escape. */
var ownerFailure = (error) => typeof error === "object" && error !== null && "code" in error && error.code === "REMOTE_SCOPE_DENIED" ? { code: "REMOTE_SCOPE_DENIED" } : unavailable();
var invalid = () => ({ code: "REMOTE_INPUT_INVALID" });
var canonicalFields = (value) => JSON.stringify(Object.fromEntries(Object.entries(value).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)));
var digest = async (bytes) => Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes))), (b) => b.toString(16).padStart(2, "0")).join("");
function createSealedSync(context, guard, fetcher, wake, ownerLive, page = { listeners: 0 }, onClose = () => {}) {
	let closed = false;
	const subscriptions = /* @__PURE__ */ new Set();
	const calls = /* @__PURE__ */ new Set();
	const live = () => {
		ownerLive();
		if (closed) throw unavailable();
	};
	const http = syncHttp(context, guard, fetcher, live);
	const base = `projects/${context.config.projectId}/databases/(default)/documents/`;
	const resource = (name) => {
		const r = context.resources.find((r) => r.name === name);
		if (!r) throw invalid();
		return r;
	};
	const invoke = async (call, action) => {
		live();
		syncCallLive(call);
		const controller = new AbortController();
		const abort = () => controller.abort();
		call.signal.addEventListener("abort", abort, { once: true });
		calls.add(controller);
		try {
			return await action({
				...call,
				signal: controller.signal
			});
		} finally {
			calls.delete(controller);
			call.signal.removeEventListener("abort", abort);
		}
	};
	const document = async (value, r) => {
		if (typeof value !== "object" || value === null || !("name" in value) || !("fields" in value) || typeof value.name !== "string" || typeof value.fields !== "object" || value.fields === null || Array.isArray(value.fields)) throw unavailable();
		const prefix = `${base}x/${context.extension}/${r.path}/`;
		if (!value.name.startsWith(prefix)) throw unavailable();
		const id = value.name.slice(prefix.length);
		const bytes = new TextEncoder().encode(canonicalFields(value.fields));
		const d = {
			target: {
				resource: r.name,
				id
			},
			bytes,
			digest: await digest(bytes)
		};
		live();
		await validateSyncDocument(context, d, "read");
		live();
		return d;
	};
	const status = (code) => {
		if (code === 401 || code === 403) throw { code: "REMOTE_SCOPE_DENIED" };
		if (code === 429) {
			const result = classifyProviderExhausted({ mutation: false }, Date.now()).outcome;
			if (result.outcome === "refused") throw {
				code: "REMOTE_BUDGET_EXHAUSTED",
				resetAtMs: result.resetAtMs,
				retryAfterMs: result.retryAfterMs
			};
		}
		throw unavailable();
	};
	const runQuery = createSyncQuery(context, http, resource, document, status, live);
	return Object.freeze({
		context,
		read(target, call) {
			const path = syncDocumentPath(context, target);
			const r = resource(target.resource);
			if (!r.sync) throw unavailable();
			return invoke(call, async (bound) => {
				const response = await http("read", path, void 0, bound, 1);
				if (response.kind === "unknown") throw unavailable();
				if (response.status === 404) return void 0;
				if (response.status !== 200) status(response.status);
				const d = await document(response.value, r);
				if (d.target.id !== target.id) throw unavailable();
				syncCallLive(bound);
				return d;
			});
		},
		query(request, call) {
			return invoke(call, (bound) => runQuery(request, bound));
		},
		async publish(mutation, call) {
			const desired = mutation.mode === "delete" ? structuredClone(mutation) : {
				mode: mutation.mode,
				document: {
					...mutation.document,
					target: { ...mutation.document.target },
					bytes: new Uint8Array(mutation.document.bytes)
				}
			};
			await validateSyncMutation(context, desired);
			live();
			syncCallLive(call);
			const target = desired.mode === "delete" ? desired.target : desired.document.target;
			const r = resource(target.resource);
			const fields = desired.mode === "delete" ? void 0 : validateSyncFields(desired.document.bytes);
			const stream = fields && r.sync.query ? fields[r.sync.query.streamField].stringValue : target.id;
			const streamKey = JSON.stringify([
				context.config.projectId,
				context.uid,
				stream
			]);
			if (tab.streams.has(streamKey) || tab.writes >= 4) throw { code: "REMOTE_RATE_LIMITED" };
			tab.streams.add(streamKey);
			tab.writes++;
			try {
				const name = `${base}${syncDocumentPath(context, target)}`;
				const write = desired.mode === "delete" ? { delete: name } : {
					update: {
						name,
						fields
					},
					...desired.mode === "create" ? { currentDocument: { exists: false } } : {}
				};
				return await invoke(call, async (bound) => {
					const response = await http("commit", "", JSON.stringify({ writes: [write] }), bound, 0, Object.freeze({
						mode: desired.mode,
						resource: target.resource,
						target: target.id
					}));
					if (response.kind === "unknown") return response;
					if (response.status === 403 && typeof response.value === "object" && response.value !== null && "error" in response.value && typeof response.value.error === "object" && response.value.error !== null && "code" in response.value.error && response.value.error.code === 403 && "status" in response.value.error && response.value.error.status === "PERMISSION_DENIED") throw { code: "REMOTE_SCOPE_DENIED" };
					if (response.status === 409 && desired.mode === "create" && typeof response.value === "object" && response.value !== null && "error" in response.value && typeof response.value.error === "object" && response.value.error !== null && "status" in response.value.error && response.value.error.status === "ALREADY_EXISTS") {
						const existing = await http("read", syncDocumentPath(context, target), void 0, bound, 1);
						if (existing.kind === "unknown" || existing.status !== 200) return { kind: "unknown" };
						const actual = await document(existing.value, r);
						live();
						syncCallLive(bound);
						if (actual.digest === desired.document.digest) return { kind: "already-present" };
						throw {
							code: "REMOTE_STATE_UNAVAILABLE",
							state: "conflict"
						};
					}
					if (response.status === 200 && typeof response.value === "object" && response.value !== null && "writeResults" in response.value && Array.isArray(response.value.writeResults) && response.value.writeResults.length === 1 && "commitTime" in response.value && typeof response.value.commitTime === "string" && Number.isFinite(Date.parse(response.value.commitTime))) return { kind: "confirmed" };
					return { kind: "unknown" };
				});
			} finally {
				tab.streams.delete(streamKey);
				tab.writes--;
			}
		},
		async subscribe(interest, call) {
			const boundInterest = structuredClone(interest);
			validateSyncInterest(context, boundInterest);
			if (new TextEncoder().encode(JSON.stringify(boundInterest.scope)).byteLength > 397312) throw invalid();
			live();
			syncCallLive(call);
			if (page.listeners >= 3 || tab.listeners >= 12) throw { code: "REMOTE_RATE_LIMITED" };
			let ended = false, dirty = false, failure;
			let pending;
			let stop;
			const publish = () => {
				if (ended || closed) return;
				try {
					live();
				} catch (error) {
					failure ??= ownerFailure(error);
				}
				if (pending) {
					const p = pending;
					pending = void 0;
					if (failure) p.reject(failure);
					else {
						dirty = false;
						p.resolve({ kind: "changed" });
					}
				}
			};
			const subscription = {
				async next(nextCall) {
					if (failure) throw failure;
					if (ended || closed) throw unavailable();
					try {
						live();
					} catch (error) {
						throw ownerFailure(error);
					}
					if (nextCall.signal.aborted) throw {
						code: "REMOTE_STATE_UNAVAILABLE",
						state: "cancelled"
					};
					if (!Number.isFinite(nextCall.deadlineAtMs)) throw unavailable();
					if (Date.now() >= nextCall.deadlineAtMs) throw {
						code: "REMOTE_STATE_UNAVAILABLE",
						state: "deadline"
					};
					if (pending) return Promise.reject({ code: "REMOTE_RATE_LIMITED" });
					if (dirty) {
						dirty = false;
						return Promise.resolve({ kind: "changed" });
					}
					return new Promise((resolve, reject) => {
						const controller = setTimeout(() => {
							pending?.reject({
								code: "REMOTE_STATE_UNAVAILABLE",
								state: "deadline"
							});
							pending = void 0;
						}, Math.max(0, nextCall.deadlineAtMs - Date.now()));
						const abort = () => {
							pending?.reject({
								code: "REMOTE_STATE_UNAVAILABLE",
								state: "cancelled"
							});
							pending = void 0;
						};
						nextCall.signal.addEventListener("abort", abort, { once: true });
						const cleanup = () => {
							clearTimeout(controller);
							nextCall.signal.removeEventListener("abort", abort);
						};
						pending = {
							resolve(change) {
								cleanup();
								resolve(change);
							},
							reject(error) {
								cleanup();
								reject(error);
							}
						};
					});
				},
				close() {
					if (ended) return;
					ended = true;
					stop?.();
					subscriptions.delete(subscription);
					tab.listeners--;
					page.listeners--;
					pending?.reject(failure ?? unavailable());
					pending = void 0;
				}
			};
			subscriptions.add(subscription);
			tab.listeners++;
			page.listeners++;
			try {
				await admitSync(guard, {
					operation: "listen",
					reads: null,
					writes: 0,
					rulesLookupsPerDocument: 3,
					listener: {
						initialTail: "all-matching-documents",
						deliveries: "including-own-writes",
						catchUp: "additional-rest-reads",
						reconnects: "fresh-query-cost"
					}
				}, call);
				live();
				syncCallLive(call);
				const credential = await syncWait(call, () => context.auth.token(call));
				live();
				syncCallLive(call);
				if ("state" in credential) throw credential;
				if (credential.projectId !== context.config.projectId || credential.uid !== context.uid || !Number.isFinite(credential.expiresAtMs) || credential.expiresAtMs <= Date.now()) throw unavailable();
				stop = await syncWait(call, async () => {
					const detach = await wake.listen(context, boundInterest, (documents) => {
						if (ended || closed || failure) return;
						const changed = () => {
							if (ended || closed || failure) return;
							dirty = true;
							publish();
						};
						const refused = (error) => {
							if (ended || closed) return;
							failure = syncGuardFailure(error);
							publish();
							subscription.close();
						};
						if (!guard.received) return changed();
						try {
							if (typeof documents !== "number" || !Number.isSafeInteger(documents) || documents < 0) throw invalid();
							guard.received(documents).then(changed, refused);
						} catch (error) {
							refused(error);
						}
					}, (error) => {
						failure = error ?? unavailable();
						publish();
						subscription.close();
					});
					if (ended || closed || call.signal.aborted || Date.now() >= call.deadlineAtMs) {
						detach();
						throw failure ?? unavailable();
					}
					return detach;
				});
				live();
				syncCallLive(call);
				if (ended) throw failure ?? unavailable();
				return subscription;
			} catch (error) {
				subscription.close();
				throw error;
			}
		},
		close() {
			if (closed) return;
			closed = true;
			for (const controller of calls) controller.abort();
			for (const s of subscriptions) s.close();
			onClose();
		}
	});
}
//#endregion
//#region src/firebase-auth.ts
/** One hosted Auth owner; provider-native answers are not local machine-signed receipts. */
var hostedPage = { listeners: 0 };
function mapped(error) {
	if (error instanceof ClientError || error instanceof RefusalError) return error;
	const code = typeof error === "object" && error !== null && "code" in error ? error.code : void 0;
	if (code === "remote/auth-pending") return new RefusalError("REMOTE_STATE_UNAVAILABLE");
	if (code === "auth/operation-not-allowed") return new RefusalError("REMOTE_SCOPE_DENIED");
	if (code === "auth/too-many-requests") return new RefusalError("REMOTE_RATE_LIMITED");
	return new ClientError("transport_failure", "Firebase authentication could not be confirmed.");
}
async function bounded$1(action) {
	const controller = new AbortController();
	let timer;
	const deadline = new Promise((_, reject) => {
		timer = setTimeout(() => {
			controller.abort();
			reject(new ClientError("timeout", "Firebase authentication check timed out."));
		}, HOSTED_BOOTSTRAP_MS);
	});
	try {
		return await Promise.race([action(controller.signal), deadline]);
	} finally {
		clearTimeout(timer);
	}
}
/** Ports are internal test edges; the browser entry supplies same-origin fetch and lazy import. */
async function connectHosted(link, declaration, deps) {
	let port;
	let verifiedConfig;
	const frozenDeclaration = { ...declaration };
	try {
		port = await bounded$1(async (signal) => {
			await verifyHostedDeclaration(frozenDeclaration);
			const config = await verifyHostedLink(link, deps.origin);
			verifiedConfig = config;
			await confirmHostedConfig(config, deps.origin, deps.fetch, signal);
			if (signal.aborted) throw new ClientError("timeout", "Hosted bootstrap timed out.");
			const module = await deps.load().catch(() => {
				throw new RefusalError("REMOTE_STATE_UNAVAILABLE");
			});
			if (signal.aborted) throw new ClientError("timeout", "Hosted bootstrap timed out.");
			const initialized = await module.initialize(config);
			if (signal.aborted) {
				await initialized.close();
				throw new ClientError("timeout", "Hosted bootstrap timed out.");
			}
			return initialized;
		});
	} catch (error) {
		throw mapped(error);
	}
	let closed = false;
	let uid;
	let generation = 0;
	const syncOwners = /* @__PURE__ */ new Set();
	let pending;
	let closing;
	const live = () => {
		if (closed) throw new RefusalError("REMOTE_CLOSED");
	};
	return Object.freeze({
		async sealedSync(space, guard) {
			live();
			if (!uid || !port.syncAuth || !port.wake) throw new RefusalError("REMOTE_STATE_UNAVAILABLE");
			if (!guard || typeof guard.admit !== "function") throw new RefusalError("REMOTE_INPUT_INVALID");
			const identity = uid, stamp = generation;
			const fence = () => {
				live();
				if (stamp !== generation || uid !== identity) throw new RefusalError("REMOTE_STATE_UNAVAILABLE");
				port.assertSyncIdentity?.(identity);
			};
			const context = await bindSyncContext(verifiedConfig, frozenDeclaration, space, port.syncAuth(identity));
			fence();
			const bound = createSealedSync(context, guard, deps.fetch, port.wake, fence, hostedPage, () => syncOwners.delete(bound));
			syncOwners.add(bound);
			return bound;
		},
		signInAnonymous() {
			live();
			if (!pending) pending = bounded$1(async (signal) => {
				try {
					const signedIn = await port.signInAnonymous();
					live();
					if (signal.aborted) throw new ClientError("timeout", "Firebase authentication check timed out.");
					if (typeof signedIn !== "string" || signedIn.length === 0 || signedIn.length > 128) throw new ClientError("unverifiable_response", "Firebase identity could not be confirmed.");
					uid = signedIn;
					return signedIn;
				} catch (error) {
					throw mapped(error);
				}
			}).finally(() => {
				pending = void 0;
			});
			return pending;
		},
		async signOut() {
			live();
			if (pending) throw new RefusalError("REMOTE_STATE_UNAVAILABLE");
			generation++;
			uid = void 0;
			for (const owner of syncOwners) owner.close();
			syncOwners.clear();
			try {
				await bounded$1(() => port.signOut());
				live();
			} catch (error) {
				throw mapped(error);
			}
		},
		close() {
			closed = true;
			generation++;
			uid = void 0;
			for (const owner of syncOwners) owner.close();
			syncOwners.clear();
			closing ??= port.close().catch((error) => {
				throw mapped(error);
			});
			return closing;
		}
	});
}
//#endregion
//#region src/session-reopen.ts
var ReopenSessionError = class extends Error {
	reason;
	detail;
	cause;
	constructor(reason, detail, cause) {
		super(`Remote session admission: ${reason} (${detail}).`);
		this.reason = reason;
		this.detail = detail;
		this.name = "ReopenSessionError";
		this.cause = cause;
	}
};
var DEADLINE = 2e4;
var ATTEMPT = 4e3;
var DELAYS = [
	250,
	500,
	1e3
];
var runtime = {
	now: () => performance.now(),
	later: (callback, ms) => setTimeout(callback, ms),
	clear: (timer) => clearTimeout(timer),
	jitter: () => crypto.getRandomValues(/* @__PURE__ */ new Uint32Array(1))[0] / 4294967296
};
function active(signal) {
	if (signal?.aborted) throw new ReopenSessionError("transient", "cancelled");
}
/** Bounds even a provider that ignores AbortSignal; abandoned work cannot publish a Session. */
function bounded(action, ms, owner, clock) {
	return new Promise((resolve, reject) => {
		const child = new AbortController();
		const end = clock.now() + ms;
		let settled = false;
		let timer;
		const finish = (value, error) => {
			if (settled) return;
			settled = true;
			if (timer !== void 0) clock.clear(timer);
			owner?.removeEventListener("abort", cancel);
			child.abort();
			if (error !== void 0) reject(error);
			else if (clock.now() >= end) reject(new ReopenSessionError("unreachable", "budget-exhausted"));
			else resolve(value);
		};
		const cancel = () => finish(void 0, new ReopenSessionError("transient", "cancelled"));
		if (owner?.aborted) return cancel();
		owner?.addEventListener("abort", cancel, { once: true });
		timer = clock.later(() => finish(void 0, new ReopenSessionError("unreachable", "budget-exhausted")), ms);
		Promise.resolve().then(() => action(child.signal)).then((value) => finish(value), (error) => finish(void 0, error));
	});
}
async function boundedAdmission(attempt, signal, clock = runtime) {
	const end = clock.now() + DEADLINE;
	let last = new ReopenSessionError("unreachable", "budget-exhausted");
	let ambiguous = false;
	for (let index = 0; index < 4; index++) {
		active(signal);
		const remaining = end - clock.now();
		if (remaining <= 0) break;
		try {
			const value = await bounded(attempt, Math.min(ATTEMPT, remaining), signal, clock);
			active(signal);
			return value;
		} catch (error) {
			if (!(error instanceof ReopenSessionError)) throw new ReopenSessionError("transient", "admission-unconfirmed");
			if ([
				"capacity",
				"mismatch",
				"unpaired",
				"revoked",
				"expired"
			].includes(error.reason) || ["cancelled", "unverifiable-response"].includes(error.detail)) throw error;
			last = error;
			ambiguous ||= error.reason === "unconfirmed";
		}
		const delay = DELAYS[index];
		if (delay === void 0) break;
		const wait = Math.min(Math.floor(delay + delay * .25 * clock.jitter()), end - clock.now());
		if (wait <= 0) break;
		try {
			await bounded((waiting) => new Promise((resolve) => {
				const timer = clock.later(() => {
					waiting.removeEventListener("abort", cancel);
					resolve();
				}, wait);
				const cancel = () => clock.clear(timer);
				waiting.addEventListener("abort", cancel, { once: true });
			}), end - clock.now(), signal, clock);
		} catch (error) {
			active(signal);
			if (!(error instanceof ReopenSessionError) || error.detail === "cancelled") throw error;
			break;
		}
	}
	active(signal);
	throw new ReopenSessionError(ambiguous ? "unconfirmed" : last.reason, "budget-exhausted");
}
//#endregion
//#region src/firestore.ts
var deploy = "tmt remote deploy firestore";
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
var firestoreSpecs = [
	["sharing", [
		"project",
		"sign-in",
		"rules",
		"indexes",
		"plan-tier",
		"quota"
	]],
	["operations", ["plan-tier", "support"]],
	["attachments", [
		"support",
		"project",
		"rules",
		"quota"
	]]
];
function requireValid(condition) {
	if (!condition) throw new Error("Invalid Firestore readiness response.");
}
function object$1(value) {
	requireValid(typeof value === "object" && value !== null && !Array.isArray(value));
	return value;
}
function keys(row, expected) {
	requireValid(Object.keys(row).length === expected.length && expected.every((key) => Object.hasOwn(row, key)));
}
function parseFirestoreLayers(value) {
	requireValid(Array.isArray(value));
	if (!value.length) return [];
	requireValid(value.length === firestoreSpecs.length);
	for (const [index, [layer, items]] of firestoreSpecs.entries()) {
		const row = object$1(value[index]);
		keys(row, [
			"layer",
			"state",
			"prerequisites"
		]);
		requireValid(row.layer === layer && Array.isArray(row.prerequisites) && row.prerequisites.length === items.length);
		const states = [];
		for (const [i, item] of items.entries()) {
			const entry = object$1(row.prerequisites[i]);
			requireValid(entry.item === item && [
				"enabled",
				"not-enabled",
				"unknown"
			].includes(entry.state));
			states.push(entry.state);
			if (entry.state === "enabled") {
				keys(entry, ["item", "state"]);
				continue;
			}
			const reason = typeof entry.reason === "string" && Object.hasOwn(firestoreReasons[item], entry.reason) ? firestoreReasons[item][entry.reason] : void 0;
			requireValid(reason && entry.state === "unknown" === (entry.reason === "not-checked"));
			requireValid(entry.reason !== "paid-plan-required" || layer === "operations");
			keys(entry, reason[1] ? [
				"item",
				"state",
				"reason",
				"next"
			] : [
				"item",
				"state",
				"reason"
			]);
			requireValid(entry.next === (reason[1] ? deploy : void 0));
		}
		const aggregate = states.includes("not-enabled") ? "not-enabled" : states.includes("unknown") ? "unknown" : "enabled";
		requireValid(row.state === aggregate);
	}
	return value;
}
//#endregion
//#region src/management.ts
var UUID$1 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
var decimal = (value) => typeof value === "string" && /^[1-9][0-9]{0,19}$/.test(value);
function valid(condition) {
	if (!condition) throw new Error("Invalid management response.");
}
function input(condition) {
	if (!condition) throw new TypeError("Invalid management input.");
}
function object(value) {
	valid(typeof value === "object" && value !== null && !Array.isArray(value));
	return value;
}
function exact(value, keys) {
	valid(Object.keys(value).length === keys.length && keys.every((key) => Object.hasOwn(value, key)));
}
var integer = (value) => Number.isSafeInteger(value) && value >= 0;
function settings(value) {
	const row = object(value);
	exact(row, [
		"open",
		"source",
		"sessionsPerDevice",
		"sessionsPerDeviceSource",
		"warning"
	]);
	valid(typeof row.open === "boolean" && ["default", "settings.json"].includes(row.source));
	valid(row.sessionsPerDevice === null || decimal(row.sessionsPerDevice));
	valid(["default", "settings.json"].includes(row.sessionsPerDeviceSource));
	valid(row.warning === null || row.warning === "settings.json could not be read; defaults apply");
	return row;
}
function device(value, activity = false) {
	const row = object(value);
	exact(row, [
		"clientId",
		"name",
		"kind",
		"issuedAtMs",
		"expiresAtMs",
		"revision",
		"revoked",
		"talkEnabled",
		...activity ? [
			"thisBrowser",
			"liveSessionCount",
			"lastActivityAtMs",
			"lastUsedAtMs"
		] : []
	]);
	valid(typeof row.clientId === "string" && UUID$1.test(row.clientId));
	valid(typeof row.name === "string" && new TextEncoder().encode(row.name).length <= 64 && row.name.trim().length > 0 && ![...row.name].some((char) => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127));
	valid([
		"browser",
		"addon",
		"cli"
	].includes(row.kind) && integer(row.issuedAtMs));
	valid(row.expiresAtMs === null || integer(row.expiresAtMs));
	valid(integer(row.revision) && row.revision > 0 && typeof row.revoked === "boolean" && typeof row.talkEnabled === "boolean");
	if (activity) valid(typeof row.thisBrowser === "boolean" && integer(row.liveSessionCount) && (row.lastActivityAtMs === null || integer(row.lastActivityAtMs)) && (row.lastUsedAtMs === null || integer(row.lastUsedAtMs)));
	return row;
}
var refusalCodes = /* @__PURE__ */ new Set([
	"REMOTE_SCOPE_DENIED",
	"REMOTE_INPUT_INVALID",
	"REMOTE_RATE_LIMITED",
	"REMOTE_INTENT_CONFLICT",
	"REMOTE_CLOSED",
	"REMOTE_SESSION_ENDED",
	"REMOTE_SESSION_EVICTED",
	"REMOTE_INPUT_TOO_LARGE",
	"REMOTE_STATE_UNAVAILABLE",
	"REMOTE_MANAGEMENT_READ_ONLY",
	"REMOTE_MANAGEMENT_UNAVAILABLE",
	"REMOTE_DEVICE_REVOKED",
	"REMOTE_DEVICE_NOT_FOUND",
	"REMOTE_SETTINGS_UNAVAILABLE",
	"REMOTE_MANAGEMENT_CAPACITY",
	"REMOTE_DEVICE_LIST_CHANGED",
	"REMOTE_DEVICE_REMOVAL_LIMIT"
]);
function outcome(value, operationId) {
	const row = object(value);
	valid(row.operationId === operationId);
	if (row.state === "committed") {
		exact(row, [
			"operationId",
			"state",
			"result",
			"sessionEnded"
		]);
		valid(typeof row.sessionEnded === "boolean");
		const result = object(row.result);
		valid(Object.keys(result).length === 1);
		if (Object.hasOwn(result, "settings")) settings(result.settings);
		else if (Object.hasOwn(result, "removed")) valid(integer(result.removed) && result.removed <= 1e3 && row.sessionEnded === false);
		else device(result.device);
	} else {
		exact(row, [
			"operationId",
			"state",
			"reason"
		]);
		if (row.state === "unknown") valid(row.reason === "effect_outcome_unconfirmed");
		else valid(row.state === "refused" && typeof row.reason === "string" && refusalCodes.has(row.reason));
	}
	return row;
}
/** Constructing this helper opens nothing. Unknown mutations are never resent.
* After Session loss, explicitly reopen once and read operation(originalId).
* Failed fresh admission changes access status; it cannot establish mutation outcome.
*/
function management(session, options = {}) {
	const call = verifiedSessionRequest(session, options);
	const clientId = channelFor(session).paired.clientId;
	async function mutate(operation, value) {
		const { operationId } = value;
		input(UUID$1.test(operationId));
		try {
			return await call(operation, operationId, value, (reply) => {
				const parsed = outcome(reply, operationId);
				if (parsed.state === "committed") {
					if (operation === "remote.settings.set") valid("settings" in parsed.result);
					else if (operation === "remote.devices.remove-others") valid("removed" in parsed.result && parsed.sessionEnded === false);
					else {
						valid("device" in parsed.result && parsed.result.device.clientId === value.clientId);
						if (operation === "remote.devices.rename") valid(parsed.result.device.name === value.name);
						if (operation === "remote.devices.revoke") valid(parsed.result.device.revoked);
						if (operation === "remote.devices.talk") valid(parsed.result.device.talkEnabled === value.enabled);
					}
				}
				return parsed;
			}, true);
		} catch (error) {
			if (error instanceof ClientError) throw new ClientError(error.code, error.message, operationId);
			if (error instanceof RefusalError) return {
				state: "refused",
				operationId,
				reason: error.code
			};
			throw error;
		}
	}
	return {
		clientId,
		previewRemoveOthers: () => call("remote.devices.remove-others.preview", crypto.randomUUID(), {}, (value) => {
			const row = object(value);
			exact(row, ["count", "fingerprint"]);
			valid(integer(row.count) && row.count <= 1e3 && typeof row.fingerprint === "string" && /^[0-9a-f]{64}$/.test(row.fingerprint));
			return row;
		}),
		removeOthers: (change) => {
			input(typeof change.fingerprint === "string" && /^[0-9a-f]{64}$/.test(change.fingerprint));
			return mutate("remote.devices.remove-others", { ...change });
		},
		settings: ((options) => call("remote.settings.show", crypto.randomUUID(), options ?? {}, (value) => {
			const row = object(value);
			exact(row, [
				"settings",
				"capabilities",
				"readOnlyReason",
				...options ? ["firestoreLayers", "firestoreBudget"] : []
			]);
			if (options) {
				parseFirestoreLayers(row.firestoreLayers);
				parsePublishedLimits(row.firestoreBudget);
			}
			settings(row.settings);
			const capabilities = object(row.capabilities);
			exact(capabilities, ["settingsWrite", "devicesWrite"]);
			valid(typeof capabilities.settingsWrite === "boolean" && typeof capabilities.devicesWrite === "boolean");
			valid(row.readOnlyReason === null || row.readOnlyReason === "local_cli_required");
			valid(capabilities.settingsWrite === capabilities.devicesWrite && capabilities.settingsWrite === (row.readOnlyReason === null));
			return row;
		})),
		devices: (page) => {
			input(Number.isInteger(page.limit) && page.limit >= 1 && page.limit <= 50 && (page.cursor === null || typeof page.cursor === "string" && /^[A-Za-z0-9_-]{98}$/.test(page.cursor)));
			return call("remote.devices.list", crypto.randomUUID(), page, (value) => {
				const row = object(value);
				exact(row, ["devices", "nextCursor"]);
				valid(Array.isArray(row.devices) && row.devices.length <= page.limit);
				const ids = /* @__PURE__ */ new Set();
				for (const entry of row.devices) {
					const item = device(entry, true);
					valid(!item.revoked && !ids.has(item.clientId));
					ids.add(item.clientId);
				}
				valid(row.nextCursor === null || typeof row.nextCursor === "string" && /^[A-Za-z0-9_-]{98}$/.test(row.nextCursor));
				return row;
			});
		},
		set: (change) => {
			input(change.setting === "open" && typeof change.value === "boolean" || change.setting === "sessions-per-device" && (change.value === null || decimal(change.value)));
			return mutate("remote.settings.set", { ...change });
		},
		rename: (change) => {
			input(UUID$1.test(change.clientId) && typeof change.name === "string" && change.name.trim().length > 0 && new TextEncoder().encode(change.name).length <= 64 && ![...change.name].some((char) => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127));
			return mutate("remote.devices.rename", { ...change });
		},
		talk: (change) => {
			input(UUID$1.test(change.clientId) && typeof change.enabled === "boolean");
			return mutate("remote.devices.talk", { ...change });
		},
		revoke: (change) => {
			input(UUID$1.test(change.clientId));
			return mutate("remote.devices.revoke", { ...change });
		},
		operation: (operationId) => {
			input(UUID$1.test(operationId));
			return call("remote.management.operation", crypto.randomUUID(), { operationId }, (value) => outcome(value, operationId));
		}
	};
}
//#endregion
//#region ../../rust/tmt-remote/assets/bip39-english.txt?raw
var bip39_english_default = "abandon\nability\nable\nabout\nabove\nabsent\nabsorb\nabstract\nabsurd\nabuse\naccess\naccident\naccount\naccuse\nachieve\nacid\nacoustic\nacquire\nacross\nact\naction\nactor\nactress\nactual\nadapt\nadd\naddict\naddress\nadjust\nadmit\nadult\nadvance\nadvice\naerobic\naffair\nafford\nafraid\nagain\nage\nagent\nagree\nahead\naim\nair\nairport\naisle\nalarm\nalbum\nalcohol\nalert\nalien\nall\nalley\nallow\nalmost\nalone\nalpha\nalready\nalso\nalter\nalways\namateur\namazing\namong\namount\namused\nanalyst\nanchor\nancient\nanger\nangle\nangry\nanimal\nankle\nannounce\nannual\nanother\nanswer\nantenna\nantique\nanxiety\nany\napart\napology\nappear\napple\napprove\napril\narch\narctic\narea\narena\nargue\narm\narmed\narmor\narmy\naround\narrange\narrest\narrive\narrow\nart\nartefact\nartist\nartwork\nask\naspect\nassault\nasset\nassist\nassume\nasthma\nathlete\natom\nattack\nattend\nattitude\nattract\nauction\naudit\naugust\naunt\nauthor\nauto\nautumn\naverage\navocado\navoid\nawake\naware\naway\nawesome\nawful\nawkward\naxis\nbaby\nbachelor\nbacon\nbadge\nbag\nbalance\nbalcony\nball\nbamboo\nbanana\nbanner\nbar\nbarely\nbargain\nbarrel\nbase\nbasic\nbasket\nbattle\nbeach\nbean\nbeauty\nbecause\nbecome\nbeef\nbefore\nbegin\nbehave\nbehind\nbelieve\nbelow\nbelt\nbench\nbenefit\nbest\nbetray\nbetter\nbetween\nbeyond\nbicycle\nbid\nbike\nbind\nbiology\nbird\nbirth\nbitter\nblack\nblade\nblame\nblanket\nblast\nbleak\nbless\nblind\nblood\nblossom\nblouse\nblue\nblur\nblush\nboard\nboat\nbody\nboil\nbomb\nbone\nbonus\nbook\nboost\nborder\nboring\nborrow\nboss\nbottom\nbounce\nbox\nboy\nbracket\nbrain\nbrand\nbrass\nbrave\nbread\nbreeze\nbrick\nbridge\nbrief\nbright\nbring\nbrisk\nbroccoli\nbroken\nbronze\nbroom\nbrother\nbrown\nbrush\nbubble\nbuddy\nbudget\nbuffalo\nbuild\nbulb\nbulk\nbullet\nbundle\nbunker\nburden\nburger\nburst\nbus\nbusiness\nbusy\nbutter\nbuyer\nbuzz\ncabbage\ncabin\ncable\ncactus\ncage\ncake\ncall\ncalm\ncamera\ncamp\ncan\ncanal\ncancel\ncandy\ncannon\ncanoe\ncanvas\ncanyon\ncapable\ncapital\ncaptain\ncar\ncarbon\ncard\ncargo\ncarpet\ncarry\ncart\ncase\ncash\ncasino\ncastle\ncasual\ncat\ncatalog\ncatch\ncategory\ncattle\ncaught\ncause\ncaution\ncave\nceiling\ncelery\ncement\ncensus\ncentury\ncereal\ncertain\nchair\nchalk\nchampion\nchange\nchaos\nchapter\ncharge\nchase\nchat\ncheap\ncheck\ncheese\nchef\ncherry\nchest\nchicken\nchief\nchild\nchimney\nchoice\nchoose\nchronic\nchuckle\nchunk\nchurn\ncigar\ncinnamon\ncircle\ncitizen\ncity\ncivil\nclaim\nclap\nclarify\nclaw\nclay\nclean\nclerk\nclever\nclick\nclient\ncliff\nclimb\nclinic\nclip\nclock\nclog\nclose\ncloth\ncloud\nclown\nclub\nclump\ncluster\nclutch\ncoach\ncoast\ncoconut\ncode\ncoffee\ncoil\ncoin\ncollect\ncolor\ncolumn\ncombine\ncome\ncomfort\ncomic\ncommon\ncompany\nconcert\nconduct\nconfirm\ncongress\nconnect\nconsider\ncontrol\nconvince\ncook\ncool\ncopper\ncopy\ncoral\ncore\ncorn\ncorrect\ncost\ncotton\ncouch\ncountry\ncouple\ncourse\ncousin\ncover\ncoyote\ncrack\ncradle\ncraft\ncram\ncrane\ncrash\ncrater\ncrawl\ncrazy\ncream\ncredit\ncreek\ncrew\ncricket\ncrime\ncrisp\ncritic\ncrop\ncross\ncrouch\ncrowd\ncrucial\ncruel\ncruise\ncrumble\ncrunch\ncrush\ncry\ncrystal\ncube\nculture\ncup\ncupboard\ncurious\ncurrent\ncurtain\ncurve\ncushion\ncustom\ncute\ncycle\ndad\ndamage\ndamp\ndance\ndanger\ndaring\ndash\ndaughter\ndawn\nday\ndeal\ndebate\ndebris\ndecade\ndecember\ndecide\ndecline\ndecorate\ndecrease\ndeer\ndefense\ndefine\ndefy\ndegree\ndelay\ndeliver\ndemand\ndemise\ndenial\ndentist\ndeny\ndepart\ndepend\ndeposit\ndepth\ndeputy\nderive\ndescribe\ndesert\ndesign\ndesk\ndespair\ndestroy\ndetail\ndetect\ndevelop\ndevice\ndevote\ndiagram\ndial\ndiamond\ndiary\ndice\ndiesel\ndiet\ndiffer\ndigital\ndignity\ndilemma\ndinner\ndinosaur\ndirect\ndirt\ndisagree\ndiscover\ndisease\ndish\ndismiss\ndisorder\ndisplay\ndistance\ndivert\ndivide\ndivorce\ndizzy\ndoctor\ndocument\ndog\ndoll\ndolphin\ndomain\ndonate\ndonkey\ndonor\ndoor\ndose\ndouble\ndove\ndraft\ndragon\ndrama\ndrastic\ndraw\ndream\ndress\ndrift\ndrill\ndrink\ndrip\ndrive\ndrop\ndrum\ndry\nduck\ndumb\ndune\nduring\ndust\ndutch\nduty\ndwarf\ndynamic\neager\neagle\nearly\nearn\nearth\neasily\neast\neasy\necho\necology\neconomy\nedge\nedit\neducate\neffort\negg\neight\neither\nelbow\nelder\nelectric\nelegant\nelement\nelephant\nelevator\nelite\nelse\nembark\nembody\nembrace\nemerge\nemotion\nemploy\nempower\nempty\nenable\nenact\nend\nendless\nendorse\nenemy\nenergy\nenforce\nengage\nengine\nenhance\nenjoy\nenlist\nenough\nenrich\nenroll\nensure\nenter\nentire\nentry\nenvelope\nepisode\nequal\nequip\nera\nerase\nerode\nerosion\nerror\nerupt\nescape\nessay\nessence\nestate\neternal\nethics\nevidence\nevil\nevoke\nevolve\nexact\nexample\nexcess\nexchange\nexcite\nexclude\nexcuse\nexecute\nexercise\nexhaust\nexhibit\nexile\nexist\nexit\nexotic\nexpand\nexpect\nexpire\nexplain\nexpose\nexpress\nextend\nextra\neye\neyebrow\nfabric\nface\nfaculty\nfade\nfaint\nfaith\nfall\nfalse\nfame\nfamily\nfamous\nfan\nfancy\nfantasy\nfarm\nfashion\nfat\nfatal\nfather\nfatigue\nfault\nfavorite\nfeature\nfebruary\nfederal\nfee\nfeed\nfeel\nfemale\nfence\nfestival\nfetch\nfever\nfew\nfiber\nfiction\nfield\nfigure\nfile\nfilm\nfilter\nfinal\nfind\nfine\nfinger\nfinish\nfire\nfirm\nfirst\nfiscal\nfish\nfit\nfitness\nfix\nflag\nflame\nflash\nflat\nflavor\nflee\nflight\nflip\nfloat\nflock\nfloor\nflower\nfluid\nflush\nfly\nfoam\nfocus\nfog\nfoil\nfold\nfollow\nfood\nfoot\nforce\nforest\nforget\nfork\nfortune\nforum\nforward\nfossil\nfoster\nfound\nfox\nfragile\nframe\nfrequent\nfresh\nfriend\nfringe\nfrog\nfront\nfrost\nfrown\nfrozen\nfruit\nfuel\nfun\nfunny\nfurnace\nfury\nfuture\ngadget\ngain\ngalaxy\ngallery\ngame\ngap\ngarage\ngarbage\ngarden\ngarlic\ngarment\ngas\ngasp\ngate\ngather\ngauge\ngaze\ngeneral\ngenius\ngenre\ngentle\ngenuine\ngesture\nghost\ngiant\ngift\ngiggle\nginger\ngiraffe\ngirl\ngive\nglad\nglance\nglare\nglass\nglide\nglimpse\nglobe\ngloom\nglory\nglove\nglow\nglue\ngoat\ngoddess\ngold\ngood\ngoose\ngorilla\ngospel\ngossip\ngovern\ngown\ngrab\ngrace\ngrain\ngrant\ngrape\ngrass\ngravity\ngreat\ngreen\ngrid\ngrief\ngrit\ngrocery\ngroup\ngrow\ngrunt\nguard\nguess\nguide\nguilt\nguitar\ngun\ngym\nhabit\nhair\nhalf\nhammer\nhamster\nhand\nhappy\nharbor\nhard\nharsh\nharvest\nhat\nhave\nhawk\nhazard\nhead\nhealth\nheart\nheavy\nhedgehog\nheight\nhello\nhelmet\nhelp\nhen\nhero\nhidden\nhigh\nhill\nhint\nhip\nhire\nhistory\nhobby\nhockey\nhold\nhole\nholiday\nhollow\nhome\nhoney\nhood\nhope\nhorn\nhorror\nhorse\nhospital\nhost\nhotel\nhour\nhover\nhub\nhuge\nhuman\nhumble\nhumor\nhundred\nhungry\nhunt\nhurdle\nhurry\nhurt\nhusband\nhybrid\nice\nicon\nidea\nidentify\nidle\nignore\nill\nillegal\nillness\nimage\nimitate\nimmense\nimmune\nimpact\nimpose\nimprove\nimpulse\ninch\ninclude\nincome\nincrease\nindex\nindicate\nindoor\nindustry\ninfant\ninflict\ninform\ninhale\ninherit\ninitial\ninject\ninjury\ninmate\ninner\ninnocent\ninput\ninquiry\ninsane\ninsect\ninside\ninspire\ninstall\nintact\ninterest\ninto\ninvest\ninvite\ninvolve\niron\nisland\nisolate\nissue\nitem\nivory\njacket\njaguar\njar\njazz\njealous\njeans\njelly\njewel\njob\njoin\njoke\njourney\njoy\njudge\njuice\njump\njungle\njunior\njunk\njust\nkangaroo\nkeen\nkeep\nketchup\nkey\nkick\nkid\nkidney\nkind\nkingdom\nkiss\nkit\nkitchen\nkite\nkitten\nkiwi\nknee\nknife\nknock\nknow\nlab\nlabel\nlabor\nladder\nlady\nlake\nlamp\nlanguage\nlaptop\nlarge\nlater\nlatin\nlaugh\nlaundry\nlava\nlaw\nlawn\nlawsuit\nlayer\nlazy\nleader\nleaf\nlearn\nleave\nlecture\nleft\nleg\nlegal\nlegend\nleisure\nlemon\nlend\nlength\nlens\nleopard\nlesson\nletter\nlevel\nliar\nliberty\nlibrary\nlicense\nlife\nlift\nlight\nlike\nlimb\nlimit\nlink\nlion\nliquid\nlist\nlittle\nlive\nlizard\nload\nloan\nlobster\nlocal\nlock\nlogic\nlonely\nlong\nloop\nlottery\nloud\nlounge\nlove\nloyal\nlucky\nluggage\nlumber\nlunar\nlunch\nluxury\nlyrics\nmachine\nmad\nmagic\nmagnet\nmaid\nmail\nmain\nmajor\nmake\nmammal\nman\nmanage\nmandate\nmango\nmansion\nmanual\nmaple\nmarble\nmarch\nmargin\nmarine\nmarket\nmarriage\nmask\nmass\nmaster\nmatch\nmaterial\nmath\nmatrix\nmatter\nmaximum\nmaze\nmeadow\nmean\nmeasure\nmeat\nmechanic\nmedal\nmedia\nmelody\nmelt\nmember\nmemory\nmention\nmenu\nmercy\nmerge\nmerit\nmerry\nmesh\nmessage\nmetal\nmethod\nmiddle\nmidnight\nmilk\nmillion\nmimic\nmind\nminimum\nminor\nminute\nmiracle\nmirror\nmisery\nmiss\nmistake\nmix\nmixed\nmixture\nmobile\nmodel\nmodify\nmom\nmoment\nmonitor\nmonkey\nmonster\nmonth\nmoon\nmoral\nmore\nmorning\nmosquito\nmother\nmotion\nmotor\nmountain\nmouse\nmove\nmovie\nmuch\nmuffin\nmule\nmultiply\nmuscle\nmuseum\nmushroom\nmusic\nmust\nmutual\nmyself\nmystery\nmyth\nnaive\nname\nnapkin\nnarrow\nnasty\nnation\nnature\nnear\nneck\nneed\nnegative\nneglect\nneither\nnephew\nnerve\nnest\nnet\nnetwork\nneutral\nnever\nnews\nnext\nnice\nnight\nnoble\nnoise\nnominee\nnoodle\nnormal\nnorth\nnose\nnotable\nnote\nnothing\nnotice\nnovel\nnow\nnuclear\nnumber\nnurse\nnut\noak\nobey\nobject\noblige\nobscure\nobserve\nobtain\nobvious\noccur\nocean\noctober\nodor\noff\noffer\noffice\noften\noil\nokay\nold\nolive\nolympic\nomit\nonce\none\nonion\nonline\nonly\nopen\nopera\nopinion\noppose\noption\norange\norbit\norchard\norder\nordinary\norgan\norient\noriginal\norphan\nostrich\nother\noutdoor\nouter\noutput\noutside\noval\noven\nover\nown\nowner\noxygen\noyster\nozone\npact\npaddle\npage\npair\npalace\npalm\npanda\npanel\npanic\npanther\npaper\nparade\nparent\npark\nparrot\nparty\npass\npatch\npath\npatient\npatrol\npattern\npause\npave\npayment\npeace\npeanut\npear\npeasant\npelican\npen\npenalty\npencil\npeople\npepper\nperfect\npermit\nperson\npet\nphone\nphoto\nphrase\nphysical\npiano\npicnic\npicture\npiece\npig\npigeon\npill\npilot\npink\npioneer\npipe\npistol\npitch\npizza\nplace\nplanet\nplastic\nplate\nplay\nplease\npledge\npluck\nplug\nplunge\npoem\npoet\npoint\npolar\npole\npolice\npond\npony\npool\npopular\nportion\nposition\npossible\npost\npotato\npottery\npoverty\npowder\npower\npractice\npraise\npredict\nprefer\nprepare\npresent\npretty\nprevent\nprice\npride\nprimary\nprint\npriority\nprison\nprivate\nprize\nproblem\nprocess\nproduce\nprofit\nprogram\nproject\npromote\nproof\nproperty\nprosper\nprotect\nproud\nprovide\npublic\npudding\npull\npulp\npulse\npumpkin\npunch\npupil\npuppy\npurchase\npurity\npurpose\npurse\npush\nput\npuzzle\npyramid\nquality\nquantum\nquarter\nquestion\nquick\nquit\nquiz\nquote\nrabbit\nraccoon\nrace\nrack\nradar\nradio\nrail\nrain\nraise\nrally\nramp\nranch\nrandom\nrange\nrapid\nrare\nrate\nrather\nraven\nraw\nrazor\nready\nreal\nreason\nrebel\nrebuild\nrecall\nreceive\nrecipe\nrecord\nrecycle\nreduce\nreflect\nreform\nrefuse\nregion\nregret\nregular\nreject\nrelax\nrelease\nrelief\nrely\nremain\nremember\nremind\nremove\nrender\nrenew\nrent\nreopen\nrepair\nrepeat\nreplace\nreport\nrequire\nrescue\nresemble\nresist\nresource\nresponse\nresult\nretire\nretreat\nreturn\nreunion\nreveal\nreview\nreward\nrhythm\nrib\nribbon\nrice\nrich\nride\nridge\nrifle\nright\nrigid\nring\nriot\nripple\nrisk\nritual\nrival\nriver\nroad\nroast\nrobot\nrobust\nrocket\nromance\nroof\nrookie\nroom\nrose\nrotate\nrough\nround\nroute\nroyal\nrubber\nrude\nrug\nrule\nrun\nrunway\nrural\nsad\nsaddle\nsadness\nsafe\nsail\nsalad\nsalmon\nsalon\nsalt\nsalute\nsame\nsample\nsand\nsatisfy\nsatoshi\nsauce\nsausage\nsave\nsay\nscale\nscan\nscare\nscatter\nscene\nscheme\nschool\nscience\nscissors\nscorpion\nscout\nscrap\nscreen\nscript\nscrub\nsea\nsearch\nseason\nseat\nsecond\nsecret\nsection\nsecurity\nseed\nseek\nsegment\nselect\nsell\nseminar\nsenior\nsense\nsentence\nseries\nservice\nsession\nsettle\nsetup\nseven\nshadow\nshaft\nshallow\nshare\nshed\nshell\nsheriff\nshield\nshift\nshine\nship\nshiver\nshock\nshoe\nshoot\nshop\nshort\nshoulder\nshove\nshrimp\nshrug\nshuffle\nshy\nsibling\nsick\nside\nsiege\nsight\nsign\nsilent\nsilk\nsilly\nsilver\nsimilar\nsimple\nsince\nsing\nsiren\nsister\nsituate\nsix\nsize\nskate\nsketch\nski\nskill\nskin\nskirt\nskull\nslab\nslam\nsleep\nslender\nslice\nslide\nslight\nslim\nslogan\nslot\nslow\nslush\nsmall\nsmart\nsmile\nsmoke\nsmooth\nsnack\nsnake\nsnap\nsniff\nsnow\nsoap\nsoccer\nsocial\nsock\nsoda\nsoft\nsolar\nsoldier\nsolid\nsolution\nsolve\nsomeone\nsong\nsoon\nsorry\nsort\nsoul\nsound\nsoup\nsource\nsouth\nspace\nspare\nspatial\nspawn\nspeak\nspecial\nspeed\nspell\nspend\nsphere\nspice\nspider\nspike\nspin\nspirit\nsplit\nspoil\nsponsor\nspoon\nsport\nspot\nspray\nspread\nspring\nspy\nsquare\nsqueeze\nsquirrel\nstable\nstadium\nstaff\nstage\nstairs\nstamp\nstand\nstart\nstate\nstay\nsteak\nsteel\nstem\nstep\nstereo\nstick\nstill\nsting\nstock\nstomach\nstone\nstool\nstory\nstove\nstrategy\nstreet\nstrike\nstrong\nstruggle\nstudent\nstuff\nstumble\nstyle\nsubject\nsubmit\nsubway\nsuccess\nsuch\nsudden\nsuffer\nsugar\nsuggest\nsuit\nsummer\nsun\nsunny\nsunset\nsuper\nsupply\nsupreme\nsure\nsurface\nsurge\nsurprise\nsurround\nsurvey\nsuspect\nsustain\nswallow\nswamp\nswap\nswarm\nswear\nsweet\nswift\nswim\nswing\nswitch\nsword\nsymbol\nsymptom\nsyrup\nsystem\ntable\ntackle\ntag\ntail\ntalent\ntalk\ntank\ntape\ntarget\ntask\ntaste\ntattoo\ntaxi\nteach\nteam\ntell\nten\ntenant\ntennis\ntent\nterm\ntest\ntext\nthank\nthat\ntheme\nthen\ntheory\nthere\nthey\nthing\nthis\nthought\nthree\nthrive\nthrow\nthumb\nthunder\nticket\ntide\ntiger\ntilt\ntimber\ntime\ntiny\ntip\ntired\ntissue\ntitle\ntoast\ntobacco\ntoday\ntoddler\ntoe\ntogether\ntoilet\ntoken\ntomato\ntomorrow\ntone\ntongue\ntonight\ntool\ntooth\ntop\ntopic\ntopple\ntorch\ntornado\ntortoise\ntoss\ntotal\ntourist\ntoward\ntower\ntown\ntoy\ntrack\ntrade\ntraffic\ntragic\ntrain\ntransfer\ntrap\ntrash\ntravel\ntray\ntreat\ntree\ntrend\ntrial\ntribe\ntrick\ntrigger\ntrim\ntrip\ntrophy\ntrouble\ntruck\ntrue\ntruly\ntrumpet\ntrust\ntruth\ntry\ntube\ntuition\ntumble\ntuna\ntunnel\nturkey\nturn\nturtle\ntwelve\ntwenty\ntwice\ntwin\ntwist\ntwo\ntype\ntypical\nugly\numbrella\nunable\nunaware\nuncle\nuncover\nunder\nundo\nunfair\nunfold\nunhappy\nuniform\nunique\nunit\nuniverse\nunknown\nunlock\nuntil\nunusual\nunveil\nupdate\nupgrade\nuphold\nupon\nupper\nupset\nurban\nurge\nusage\nuse\nused\nuseful\nuseless\nusual\nutility\nvacant\nvacuum\nvague\nvalid\nvalley\nvalve\nvan\nvanish\nvapor\nvarious\nvast\nvault\nvehicle\nvelvet\nvendor\nventure\nvenue\nverb\nverify\nversion\nvery\nvessel\nveteran\nviable\nvibrant\nvicious\nvictory\nvideo\nview\nvillage\nvintage\nviolin\nvirtual\nvirus\nvisa\nvisit\nvisual\nvital\nvivid\nvocal\nvoice\nvoid\nvolcano\nvolume\nvote\nvoyage\nwage\nwagon\nwait\nwalk\nwall\nwalnut\nwant\nwarfare\nwarm\nwarrior\nwash\nwasp\nwaste\nwater\nwave\nway\nwealth\nweapon\nwear\nweasel\nweather\nweb\nwedding\nweekend\nweird\nwelcome\nwest\nwet\nwhale\nwhat\nwheat\nwheel\nwhen\nwhere\nwhip\nwhisper\nwide\nwidth\nwife\nwild\nwill\nwin\nwindow\nwine\nwing\nwink\nwinner\nwinter\nwire\nwisdom\nwise\nwish\nwitness\nwolf\nwoman\nwonder\nwood\nwool\nword\nwork\nworld\nworry\nworth\nwrap\nwreck\nwrestle\nwrist\nwrite\nwrong\nyard\nyear\nyellow\nyou\nyoung\nyouth\nzebra\nzero\nzone\nzoo\n";
//#endregion
//#region src/device.ts
/**
* Device-side SDK for remote-channel-v1: the device key, the pairing ceremony,
* `session.open` and `tmt-ext-cert-v1`. Network access goes through an injected
* fetch; key persistence is the caller's (the browser page stores the opaque
* CryptoKey in IndexedDB by structured clone).
*/
function requireValue(condition, label) {
	if (!condition) throw new Error(`Invalid ${label}.`);
}
var PairingRefusedError = class extends Error {
	constructor() {
		super("Pairing was refused or ended.");
	}
};
/** A fresh ArrayBuffer-backed copy, as WebCrypto requires. */
var owned = (bytes) => new Uint8Array(bytes);
var utf8 = new TextEncoder();
var strictUtf8 = new TextDecoder("utf-8", { fatal: true });
var UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
var HEX16 = /^[0-9a-f]{32}$/;
function hex(bytes) {
	return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}
function fromHex(text) {
	requireValue(HEX16.test(text), "hex");
	return Uint8Array.from(text.match(/../g), (pair) => parseInt(pair, 16));
}
function random(length) {
	return crypto.getRandomValues(new Uint8Array(length));
}
/** Strict base64url of unknown length: the decoded length follows from the text. */
function decode(text) {
	return base64urlBytes(text, Math.floor(text.length * 6 / 8));
}
async function hmac(key, message) {
	const native = await crypto.subtle.importKey("raw", owned(key), {
		name: "HMAC",
		hash: "SHA-256"
	}, false, ["sign"]);
	return new Uint8Array(await crypto.subtle.sign("HMAC", native, owned(message)));
}
/** Constant-time HMAC check through WebCrypto. */
async function verifyHmac(key, message, tag) {
	const native = await crypto.subtle.importKey("raw", owned(key), {
		name: "HMAC",
		hash: "SHA-256"
	}, false, ["verify"]);
	return tag.length === 32 && crypto.subtle.verify("HMAC", native, owned(tag), owned(message));
}
/** A device's Ed25519 key. The private half never leaves its non-extractable CryptoKey. */
var DeviceKey = class DeviceKey {
	#private;
	#public;
	constructor(privateKey, publicKey) {
		this.#private = privateKey;
		this.#public = new Uint8Array(publicKey);
	}
	static async generate() {
		const pair = await crypto.subtle.generateKey("Ed25519", false, ["sign", "verify"]);
		const publicKey = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
		return new DeviceKey(pair.privateKey, publicKey);
	}
	/** Restore a stored handle; it must be a non-extractable Ed25519 signing key for `publicKey`. */
	static async fromHandle(privateKey, publicKey) {
		requireValue(privateKey.type === "private" && !privateKey.extractable && privateKey.algorithm.name === "Ed25519" && privateKey.usages.includes("sign") && publicKey.length === 32, "device key handle");
		const key = new DeviceKey(privateKey, publicKey);
		const probe = utf8.encode("tmt-device-key-probe");
		requireValue(await verifyEd25519(publicKey, probe, await key.sign(probe)), "device key pair");
		return key;
	}
	publicKey() {
		return new Uint8Array(this.#public);
	}
	/** The opaque handle for structured-clone storage; never exported. */
	handle() {
		return this.#private;
	}
	async sign(message) {
		return new Uint8Array(await crypto.subtle.sign("Ed25519", this.#private, owned(message)));
	}
};
var DESCRIPTOR_FIELDS = [
	"profile",
	"binding",
	"machineId",
	"windowId",
	"offerId",
	"address",
	"serverChallenge"
];
function pairingUrl(link) {
	const url = new URL(link);
	requireValue(url.protocol === "http:" && url.hostname === "127.0.0.1" && url.pathname === "/remote/pair" && url.search === "" && url.username === "" && url.password === "" && /^[A-Z2-7]{26}$/.test(url.hash.slice(1)), "pairing link");
	pairingCode(url.hash.slice(1));
	return url;
}
/** Parse a short link and validate the separately obtained public offer descriptor. */
function parseLink(link, value) {
	const url = pairingUrl(link);
	requireValue(value !== null && typeof value === "object" && !Array.isArray(value), "pairing descriptor");
	const descriptor = value;
	requireValue(Object.keys(descriptor).length === DESCRIPTOR_FIELDS.length && DESCRIPTOR_FIELDS.every((field) => typeof descriptor[field] === "string") && descriptor.profile === "local-v1" && descriptor.binding === "loopback-http" && [
		"machineId",
		"windowId",
		"offerId"
	].every((id) => UUID.test(descriptor[id])) && HEX16.test(descriptor.serverChallenge) && descriptor.address.startsWith(`${url.origin}/r/`) && /^[a-z2-7]{16}$/.test(descriptor.address.slice(`${url.origin}/r/`.length)), "pairing descriptor");
	return {
		descriptor,
		code: pairingCode(url.hash.slice(1))
	};
}
/** Fetch only the public offer; the code never enters a request URL or body here. */
async function resolveLink(link, network = fetch) {
	const response = await network(`${pairingUrl(link).origin}/sdk/pair-offer`, { cache: "no-store" });
	requireValue(response.status === 200, "pairing offer");
	return parseLink(link, await response.json());
}
/** Native-only automatic launch fragment; the public descriptor still contains no code. */
async function resolveAutomaticLink(link, network = fetch) {
	const launch = new URL(link);
	requireValue(/^#auto=[A-Z2-7]{26}$/.test(launch.hash), "automatic pairing link");
	launch.hash = launch.hash.slice(6);
	const url = pairingUrl(launch.href);
	const response = await network(`${url.origin}/sdk/pair-offer`, {
		cache: "no-store",
		redirect: "error",
		referrerPolicy: "no-referrer"
	});
	if (response.status === 404) throw new PairingRefusedError();
	requireValue(response.status === 200, "automatic pairing offer");
	const value = await response.json();
	requireValue(value !== null && typeof value === "object" && !Array.isArray(value), "automatic pairing descriptor");
	const { automatic, target, ...descriptor } = value;
	if (automatic === void 0 && target === void 0) {
		parseLink(url.href, descriptor);
		throw new PairingRefusedError();
	}
	requireValue(automatic === true && typeof target === "string" && target.startsWith("/") && !target.startsWith("//"), "automatic pairing target");
	const destination = new URL(target, url.origin);
	requireValue(destination.origin === url.origin && destination.pathname === target && !destination.search && !destination.hash, "automatic pairing target");
	return {
		...parseLink(url.href, descriptor),
		target
	};
}
async function enrollmentRequest(options, cancel) {
	const { descriptor, code, key } = options;
	const publicKey = key.publicKey();
	const clientNonce = random(16);
	const enrollment = enrollmentSigningBytes({
		profile: "local-v1",
		machineId: descriptor.machineId,
		windowId: descriptor.windowId,
		offerId: descriptor.offerId,
		serverChallenge: fromHex(descriptor.serverChallenge),
		clientNonce,
		kind: options.kind,
		origin: options.origin,
		name: options.name,
		publicKey
	});
	const mac = await hmac(code, enrollment);
	const signature = await key.sign(cancel ? enrollmentCancellationSigningBytes(enrollment, mac) : enrollmentPossessionSigningBytes(enrollment, mac));
	return {
		enrollment,
		body: JSON.stringify({
			profile: "local-v1",
			machineId: descriptor.machineId,
			windowId: descriptor.windowId,
			offerId: descriptor.offerId,
			serverChallenge: descriptor.serverChallenge,
			clientNonce: hex(clientNonce),
			kind: options.kind,
			origin: options.origin,
			name: options.name,
			publicKey: base64url(publicKey),
			mac: base64url(mac),
			signature: base64url(signature),
			...cancel ? { cancel: true } : {}
		})
	};
}
/** Cancellation is a code-bound possession proof from a currently live paired key. */
async function cancelAutomaticOffer(options) {
	const { body } = await enrollmentRequest(options, true);
	options.signal?.throwIfAborted();
	const reply = await (options.fetch ?? fetch)(`${options.descriptor.address}/pair`, {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body,
		signal: options.signal
	});
	if (reply.status === 404) throw new PairingRefusedError();
	requireValue(reply.status === 204, "automatic offer cancellation");
}
/** Submit one candidate; manual pending replies retry exactly until the owner's decision.
* Accept the machine key/grant only after serverProof verifies the exact receipt bytes. */
async function pair(options) {
	const { descriptor, code, key } = options;
	const send = options.fetch ?? fetch;
	const publicKey = key.publicKey();
	const { enrollment, body } = await enrollmentRequest(options, false);
	const deadline = Date.now() + (options.deadlineMs ?? 6e5);
	for (;;) {
		options.signal?.throwIfAborted();
		const response = await send(`${descriptor.address}/pair`, {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body,
			signal: options.signal
		});
		if (response.status === 202 && Date.now() < deadline) continue;
		if (response.status === 404) throw new PairingRefusedError();
		if (response.status !== 200) throw new Error("Pairing did not answer.");
		const reply = await response.json();
		requireValue(typeof reply.receipt === "string" && typeof reply.serverProof === "string", "pairing reply");
		const receipt = decode(reply.receipt);
		requireValue(await verifyHmac(await hmac(code, responseKeyInput(enrollment)), serverProofInput(receipt), decode(reply.serverProof)), "server proof");
		return accept(receipt, descriptor, options, publicKey);
	}
}
/** Check the proven receipt names this machine, this device and a well-formed grant. */
function accept(receipt, descriptor, options, publicKey) {
	const parsed = JSON.parse(strictUtf8.decode(receipt));
	const grant = parsed.grant ?? {};
	requireValue(typeof parsed.machinePublicKey === "string" && typeof grant.clientId === "string" && UUID.test(grant.clientId) && grant.machineId === descriptor.machineId && grant.profile === "local-v1" && grant.publicKey === base64url(publicKey) && grant.kind === options.kind && grant.origin === options.origin && Number.isSafeInteger(grant.revision) && grant.disabled === false, "receipt");
	return {
		clientId: grant.clientId,
		machineId: descriptor.machineId,
		machinePublicKey: base64urlBytes(parsed.machinePublicKey, 32),
		kind: options.kind,
		origin: options.origin,
		address: descriptor.address,
		grantRevision: grant.revision
	};
}
/**
* Open the device's session for the current remote run. `windowId` names that
* run; a browser on the door receives its session cookie with the response.
*/
async function openSession(paired, key, windowId, send = fetch, options) {
	const id = crypto.randomUUID();
	const payload = utf8.encode(JSON.stringify({ clientNonce: hex(random(16)) }));
	const control = {
		kind: "control",
		id,
		correlationId: null,
		machineId: paired.machineId,
		windowId,
		clientId: paired.clientId,
		sessionId: "new",
		sequence: "0",
		timestampMs: Date.now(),
		origin: paired.origin,
		operation: "session.open"
	};
	const signature = await key.sign(await envelopeSigningBytes({
		version: 1,
		profile: "local-v1",
		...control,
		payload
	}));
	const response = await send(`${paired.address}/append`, {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		credentials: "same-origin",
		body: JSON.stringify({
			version: 1,
			profile: "local-v1",
			...control,
			payload: base64url(payload),
			signature: base64url(signature)
		})
	});
	if (response.status !== 200) throw new Error("The session was refused.");
	const reply = await verifyResponse(await response.json(), paired, {
		id,
		windowId,
		operation: "session.open",
		after: 0n
	});
	requireValue(reply.sequence === 1n, "session response sequence");
	const decoded = JSON.parse(strictUtf8.decode(reply.payload));
	requireValue(typeof decoded === "object" && decoded !== null && !Array.isArray(decoded), "session decoded");
	if (Object.hasOwn(decoded, "error")) {
		requireValue(reply.sessionId === "new", "rejected-open session");
		const refusal = remoteError(decoded, paired.address);
		requireValue(refusal instanceof RefusalError, "rejected-open refusal");
		throw refusal;
	}
	const session = decoded;
	requireValue(UUID.test(session.sessionId) && session.sessionId === reply.sessionId && Number.isSafeInteger(session.serverTimeMs) && session.serverTimeMs >= 0 && Number.isSafeInteger(session.grantRevision) && session.grantRevision > 0 && (session.expiresAtMs === null || Number.isSafeInteger(session.expiresAtMs) && session.expiresAtMs >= 0), "session payload");
	options?.signal.throwIfAborted();
	registerChannel(session, paired, key, windowId, options?.transport ?? send);
	return session;
}
/** Certify an extension key with the device key. Callers fix `extension` from the door's mount. */
async function certify(key, value, issuedAtMs = Date.now()) {
	const signature = await key.sign(extCertSigningBytes({
		...value,
		issuedAtMs
	}));
	return {
		extension: value.extension,
		purpose: value.purpose,
		publicKey: base64url(value.publicKey),
		issuedAtMs,
		signature: base64url(signature)
	};
}
//#endregion
//#region src/browser.ts
/** Explicit hosted bootstrap; ordinary local SDK imports never load Firebase. */
var firebaseRemote = {
	originLink() {
		return originLink(location.origin, globalThis.fetch.bind(globalThis));
	},
	connect(link, declaration) {
		return connectHosted(link, declaration, {
			origin: location.origin,
			fetch: globalThis.fetch.bind(globalThis),
			load: () => import(
				/* @vite-ignore */
				"/sdk/remote-firebase-v1.js"
)
		});
	}
};
/**
* Browser entry of the device SDK, served by the door as `/sdk/remote-v1.js`.
* On the pairing page it runs the ceremony; mounted extension pages import it
* to reopen the door session and certify their own extension keys. The device
* key lives in this origin's IndexedDB as an opaque, non-extractable CryptoKey.
*/
var WORDS = bip39_english_default.split("\n").slice(0, 2048);
var RECORD = "device";
async function load() {
	const objects = await deviceObjects("readonly");
	return await request(() => objects.get(RECORD));
}
async function save(record) {
	const objects = await deviceObjects("readwrite");
	await request(() => objects.put(record, RECORD));
}
/** A stored identity is retained unless the pinned machine proves it is dead. */
async function existingPairing(signal) {
	let record;
	try {
		record = await load();
	} catch {
		throw new ReopenSessionError("transient", "pairing-unavailable");
	}
	active(signal);
	if (record === void 0) return void 0;
	try {
		await reopenSession(void 0, {
			retry: "bounded",
			signal
		});
		return record;
	} catch (error) {
		if (error instanceof ReopenSessionError && (error.reason === "revoked" || error.reason === "expired")) return void 0;
		throw error;
	}
}
function pairingRecovery(error) {
	if (error instanceof ReopenSessionError && error.detail === "pairing-unavailable") return "Your saved pairing could not be read. Check browser storage and try again.";
	if (error instanceof ReopenSessionError && error.reason === "capacity") return "This browser has reached your machine's connection limit. Close another tab, or change the limit on your machine. Your pairing is unchanged.";
	return "Remote may have stopped, or its reply couldn't be verified. Your pairing is unchanged.";
}
async function door(send = fetch) {
	const response = await send("/sdk/mount", {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ path: location.pathname })
	});
	if (response.status !== 200) throw new Error("The door did not answer.");
	return await response.json();
}
async function paired(signal) {
	let record;
	try {
		record = await load();
	} catch (error) {
		if (signal) throw new ReopenSessionError("transient", "pairing-unavailable");
		throw error;
	}
	active(signal);
	if (signal && record === void 0) throw new ReopenSessionError("unpaired", "pairing-unavailable");
	if (!record) throw new Error("This browser is not paired.");
	return {
		record,
		key: await DeviceKey.fromHandle(record.handle, record.publicKey)
	};
}
var admissions = /* @__PURE__ */ new WeakMap();
/**
* Reopen this browser's door session for the running remote; no owner step.
* The first caller's signal governs a coalesced bounded series; joined callers' signals are ignored.
* A signal without bounded retry selects one typed attempt, bounded by the caller's cancellation.
*/
function reopenSession(previous, options) {
	if (options?.retry !== "bounded") return reopenOnce(previous, options?.signal);
	if (previous && !channelFor(previous)) return Promise.reject(/* @__PURE__ */ new TypeError("Use a verified previous Session."));
	const joined = previous && admissions.get(previous);
	if (joined) return joined;
	const pending = boundedAdmission((signal) => reopenOnce(previous, signal), options.signal);
	if (previous) {
		admissions.set(previous, pending);
		pending.finally(() => {
			if (admissions.get(previous) === pending) admissions.delete(previous);
		}).catch(() => {});
	}
	return pending;
}
async function reopenOnce(previous, signal) {
	active(signal);
	const send = async (input, init) => {
		active(signal);
		let response;
		try {
			response = await fetch(input, {
				...init,
				signal
			});
		} catch (error) {
			active(signal);
			if (signal) throw new ReopenSessionError("unreachable", "admission-unconfirmed");
			throw error;
		}
		active(signal);
		if (signal && response.status !== 200) throw new ReopenSessionError("unconfirmed", "admission-unconfirmed");
		return response;
	};
	let stored;
	try {
		stored = await paired(signal);
	} catch (error) {
		active(signal);
		if (signal) {
			if (error instanceof ReopenSessionError) throw error;
			throw new ReopenSessionError("transient", "pairing-unavailable");
		}
		throw error;
	}
	active(signal);
	const { record, key } = stored;
	if (signal && (record.paired.origin !== location.origin || record.paired.kind !== "browser")) throw new ReopenSessionError("mismatch", "identity-mismatch");
	if (previous) {
		const old = channelFor(previous);
		if (!old) throw new TypeError("Use a verified previous Session.");
		if (!sameIdentity(record, old.paired, old.key.publicKey())) {
			if (signal) throw new ReopenSessionError("mismatch", "identity-mismatch");
			throw new RefusalError("REMOTE_SESSION_ENDED");
		}
	}
	const current = await door(send);
	try {
		validateDoor(current, record);
	} catch (error) {
		if (signal) throw new ReopenSessionError(current.machineId !== record.paired.machineId ? "mismatch" : "transient", current.machineId !== record.paired.machineId ? "identity-mismatch" : "unverifiable-response");
		throw error;
	}
	let refused = false;
	try {
		const replacement = await openSession({
			...record.paired,
			address: current.address
		}, key, current.windowId, async (url, init) => {
			const response = await send(url, init);
			refused = response.status === 404;
			return response;
		}, signal ? {
			signal,
			transport: fetch
		} : void 0);
		if (signal) {
			const latest = await paired(signal);
			active(signal);
			if (!sameIdentity(latest.record, record.paired, key.publicKey())) throw new ReopenSessionError("mismatch", "identity-mismatch");
		}
		return replacement;
	} catch (error) {
		active(signal);
		if (signal) {
			if (error instanceof ReopenSessionError) throw error;
			if (error instanceof RefusalError) {
				if (error.code === "REMOTE_SESSION_LIMIT") throw new ReopenSessionError("capacity", "admission-refused", error);
				if (error.code === "REMOTE_DEVICE_REVOKED" || error.code === "REMOTE_GRANT_EXPIRED") throw new ReopenSessionError(error.code === "REMOTE_DEVICE_REVOKED" ? "revoked" : "expired", "admission-refused", error);
			}
			throw new ReopenSessionError("transient", "unverifiable-response");
		}
		if (!refused || !previous) throw error;
		const latest = await door();
		validateDoor(latest, record);
		if (latest.address !== current.address || latest.windowId !== current.windowId) throw new Error("The door descriptor changed during admission.");
		throw new RefusalError("REMOTE_SESSION_ENDED");
	}
}
function sameIdentity(record, pinned, publicKey) {
	const pin = record.paired.machinePublicKey;
	return record.paired.clientId === pinned.clientId && record.paired.machineId === pinned.machineId && record.paired.origin === pinned.origin && pin instanceof Uint8Array && pin.length === 32 && pinned.machinePublicKey.length === 32 && pin.every((byte, index) => byte === pinned.machinePublicKey[index]) && record.publicKey.length === publicKey.length && record.publicKey.every((byte, index) => byte === publicKey[index]);
}
function validateDoor(current, record) {
	if (current.machineId !== record.paired.machineId) throw new Error("Paired with another machine.");
	if (typeof current.windowId !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(current.windowId) || typeof current.address !== "string" || !current.address.startsWith(`${location.origin}/r/`) || !/^[a-z2-7]{16}$/.test(current.address.slice(`${location.origin}/r/`.length))) throw new Error("The door advertised an invalid descriptor.");
}
/** Local evidence only: never read a descriptor or attempt admission to paint the entry. */
async function entryPairing() {
	try {
		const record = await load();
		if (record === void 0) return { state: "missing" };
		if (typeof record !== "object" || record === null || Array.isArray(record)) return { state: "unavailable" };
		const pin = record.paired?.machinePublicKey;
		if (typeof record.paired !== "object" || record.paired === null || Array.isArray(record.paired) || record.paired.kind !== "browser" || record.paired.origin !== location.origin || !entryUuid(record.paired.machineId) || !entryUuid(record.paired.clientId) || !entryAddress(record.paired.address) || !Number.isSafeInteger(record.paired.grantRevision) || record.paired.grantRevision < 1 || !(record.publicKey instanceof Uint8Array) || record.publicKey.length !== 32 || !(pin instanceof Uint8Array) || pin.length !== 32) return { state: "unavailable" };
		return {
			state: "saved",
			record,
			key: await DeviceKey.fromHandle(record.handle, record.publicKey)
		};
	} catch {
		return { state: "unavailable" };
	}
}
/** Landing-page observation uses the verified Session lane; it never dispatches work. */
/** `lifetime` is the route's: once it aborts the page paints nothing and starts nothing more. */
async function landingPage(lifetime) {
	if (document.readyState === "loading") await new Promise((resolve) => document.addEventListener("DOMContentLoaded", () => resolve(), { once: true }));
	const registryState = remoteRegistryPage();
	const button = element("check");
	let attempt = 0;
	let departed = false;
	let session;
	let opaque = false;
	const pageAlive = () => !departed && button.isConnected;
	lifetime?.addEventListener("abort", () => {
		departed = true;
		attempt++;
	}, { once: true });
	const commandActions = [
		["missing", "pair"],
		["different", "pair"],
		["refused", "devices"],
		["refused", "pair"],
		["unconfirmed", "status"],
		["unreadable", "pair"]
	];
	const render = (state, validated = false, refusalCode) => {
		const authorityEnded = ["REMOTE_DEVICE_REVOKED", "REMOTE_GRANT_EXPIRED"].includes(refusalCode ?? "");
		const copy = {
			missing: [
				"",
				"Not paired",
				"Pair this browser with your machine",
				""
			],
			checking: [
				"",
				"Checking",
				"Checking the connection…",
				"This browser is paired. Nothing is sent while checking."
			],
			connected: [
				"",
				"Connected",
				"This browser can use Remote",
				"Open your app from its link in this browser."
			],
			different: [
				"",
				"Different machine",
				"This browser is paired with another machine",
				"This Remote runs on a machine this browser isn't paired with. Your existing pairing is kept."
			],
			refused: [
				"",
				"Not accepted",
				"Remote didn't accept this browser",
				refusalCode === "REMOTE_SESSION_LIMIT" ? "This browser has reached your machine's connection limit. Close another tab, or change the limit on your machine. Your pairing is unchanged." : authorityEnded ? "This pairing was removed or has expired. To use Remote here, run tmt remote pair on the machine." : "This page's connection ended. Try again. Your pairing is unchanged."
			],
			unconfirmed: [
				"",
				"Can't reach Remote",
				"Couldn't confirm the connection",
				"Remote may have stopped, or its reply couldn't be verified. Your pairing is unchanged."
			],
			unreadable: [
				"",
				"Pairing unreadable",
				"This browser's pairing can't be read",
				"Your saved pairing could not be read. Check browser storage and try again."
			]
		}[state];
		element("notice").dataset.tone = "neutral";
		element("notice").dataset.state = state;
		registryState(authorityEnded ? "Access ended" : copy[1], element("protocol-address").textContent ?? void 0);
		for (const [copyState, command] of commandActions) element(`copy-feedback-${copyState}-${command}`).textContent = "";
		element("mark").textContent = copy[0];
		element("state-label").textContent = copy[1];
		element("heading").textContent = copy[2];
		element("status").textContent = state === "checking" && !validated ? "Reading this browser's pairing." : copy[3];
		if (state === "refused" && authorityEnded) {
			const command = document.createElement("code");
			command.className = "tmt-ui-code";
			command.textContent = "tmt remote pair";
			const [before, after] = copy[3].split(command.textContent);
			element("status").replaceChildren(document.createTextNode(before), command, document.createTextNode(after));
		}
		for (const name of [
			"missing",
			"different",
			"refused",
			"unconfirmed",
			"unreadable"
		]) element(`steps-${name}`).hidden = name !== state || name === "refused" && !authorityEnded || name === "unreadable";
		element("status").hidden = state === "missing";
		element("pairing-note").hidden = state !== "missing";
		element("command-location").hidden = ["checking", "connected"].includes(state);
		button.hidden = [
			"missing",
			"different",
			"unreadable"
		].includes(state);
		button.disabled = state === "checking";
	};
	for (const [state, command] of commandActions) element(`copy-${state}-${command}`).addEventListener("click", async () => {
		const text = `tmt remote ${command}`;
		try {
			await navigator.clipboard.writeText(text);
			if (pageAlive()) element(`copy-feedback-${state}-${command}`).textContent = "Copied.";
		} catch {
			if (pageAlive()) element(`copy-feedback-${state}-${command}`).textContent = "Copy failed. Select the command and copy it manually.";
		}
	});
	const check = async () => {
		const restoreFocus = document.activeElement === button;
		const currentAttempt = ++attempt;
		const currentPage = () => pageAlive() && attempt === currentAttempt;
		render("checking");
		element("checked-time").textContent = "Not checked";
		const local = await entryPairing();
		if (!currentPage()) return;
		if (local.state !== "saved") {
			session = void 0;
			render(local.state === "missing" ? "missing" : "unreadable");
			return;
		}
		render("checking", true);
		element("machine-id").textContent = local.record.paired.machineId.slice(0, 8);
		element("protocol-address").textContent = local.record.paired.address;
		element("trust-pin").textContent = Array.from(local.record.paired.machinePublicKey, (byte) => byte.toString(16).padStart(2, "0")).join("");
		const step = { phase: "open" };
		const channel = session && channelFor(session);
		const reused = !!channel && !channel.ended && channel.paired.clientId === local.record.paired.clientId && channel.paired.machineId === local.record.paired.machineId && channel.paired.machinePublicKey.every((byte, index) => byte === local.record.paired.machinePublicKey[index]) && channel.key.publicKey().every((byte, index) => byte === local.record.publicKey[index]);
		const open = async () => {
			step.phase = "open";
			opaque = false;
			session = void 0;
			const current = await door();
			if (!currentPage()) return false;
			validateEntryDoor(current);
			element("protocol-address").textContent = current.address;
			if (current.machineId !== local.record.paired.machineId) {
				render("different");
				return false;
			}
			try {
				session = await openSession({
					...local.record.paired,
					address: current.address
				}, local.key, current.windowId, async (url, init) => {
					if (!pageAlive()) throw new Error("Remote page ended.");
					const response = await fetch(url, init);
					opaque = response.status === 404;
					return response;
				});
			} catch (error) {
				if (!currentPage()) return false;
				if (opaque) {
					const latest = await door();
					if (!currentPage()) return false;
					validateEntryDoor(latest);
				}
				throw error;
			}
			return currentPage();
		};
		const observe = async () => {
			step.phase = "capabilities";
			await verifiedSessionRequest(session)("capabilities", crypto.randomUUID(), {}, parseCapabilities);
		};
		opaque = false;
		try {
			if (!reused && !await open()) return;
			if (!currentPage()) return;
			try {
				await observe();
			} catch (error) {
				if (!currentPage()) return;
				if (!reused || !(error instanceof RefusalError) || !["REMOTE_SESSION_ENDED", "REMOTE_SESSION_EVICTED"].includes(error.code)) throw error;
				if (!await open()) return;
				await observe();
			}
			if (!currentPage()) return;
			element("checked-time").textContent = new Intl.DateTimeFormat(void 0, {
				dateStyle: "medium",
				timeStyle: "short"
			}).format(/* @__PURE__ */ new Date());
			render("connected");
		} catch (error) {
			if (!currentPage()) return;
			const verifiedRefusal = !opaque && error instanceof RefusalError && !(step.phase === "capabilities" && (error.code === "REMOTE_SESSION_ENDED" || reused && error.code === "REMOTE_SESSION_EVICTED")) && [
				"REMOTE_SESSION_ENDED",
				"REMOTE_SESSION_EVICTED",
				"REMOTE_DEVICE_REVOKED",
				"REMOTE_GRANT_EXPIRED"
			].includes(error.code);
			session = void 0;
			render(verifiedRefusal ? "refused" : "unconfirmed", false, error instanceof RefusalError ? error.code : void 0);
		} finally {
			if (currentPage() && restoreFocus && !button.hidden) button.focus();
		}
	};
	button.addEventListener("click", () => {
		if (!button.disabled) check();
	});
	await check();
}
function entryUuid(value) {
	return typeof value === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value);
}
function entryAddress(value) {
	return typeof value === "string" && value.startsWith(`${location.origin}/r/`) && /^[a-z2-7]{16}$/.test(value.slice(`${location.origin}/r/`.length));
}
function validateEntryDoor(current) {
	if (!entryAddress(current.address) || !entryUuid(current.windowId) || !entryUuid(current.machineId)) throw new Error("Invalid door descriptor.");
}
/**
* Certify a key of the calling page's own extension. The extension comes from
* the door's mount mapping for this page, never from the caller. Each call
* signs a new certificate with the current issuedAtMs; verifiers own freshness.
*/
async function certifyKey(purpose, publicKey) {
	const { key } = await paired();
	const { extension } = await door();
	if (extension === null) throw new Error("Only a mounted extension page can certify keys.");
	return certify(key, {
		extension,
		purpose,
		publicKey
	});
}
function element(id) {
	const found = document.getElementById(id);
	if (!found) throw new Error(`The Remote page lacks #${id}.`);
	return found;
}
/** Present the ceremony state without coloring its instruction text. */
function showState(status, state, text) {
	status.dataset.state = state;
	element("notice").dataset.tone = {
		waiting: "waiting",
		paired: "working",
		blocked: "blocked"
	}[state];
	element("state-label").textContent = {
		waiting: "Waiting",
		paired: "Paired",
		blocked: "Unavailable"
	}[state];
	status.textContent = text;
	element("mark").textContent = {
		waiting: "◆",
		paired: "✓",
		blocked: "✗"
	}[state];
}
/** Called by the fragment-erasing bootstrap after the page DOM is ready. */
var pairingAttempt;
var manualPairingText = /* @__PURE__ */ new WeakMap();
async function pairingPage(link) {
	pairingAttempt?.abort();
	const lifetime = new AbortController();
	pairingAttempt = lifetime;
	globalThis.addEventListener?.("pagehide", () => lifetime.abort(), {
		once: true,
		signal: lifetime.signal
	});
	if (document.readyState === "loading") await new Promise((resolve) => document.addEventListener("DOMContentLoaded", () => resolve(), { once: true }));
	if (lifetime.signal.aborted) return;
	const automatic = link.includes("#auto=");
	for (const id of ["pairing-title", "heading"]) {
		const node = element(id);
		if (!manualPairingText.has(node)) manualPairingText.set(node, node.textContent ?? "");
		node.textContent = automatic ? "Connect this browser" : manualPairingText.get(node);
	}
	element("pairing-eyebrow").hidden = automatic;
	if (automatic) return automaticPairingPage(link, lifetime);
	const status = element("status");
	const form = element("pair");
	const button = form.querySelector("button");
	button.disabled = true;
	form.hidden = true;
	try {
		if (await existingPairing(lifetime.signal)) {
			active(lifetime.signal);
			showState(status, "paired", "This browser is already paired.");
			location.replace("/remote/");
			return;
		}
	} catch (error) {
		if (lifetime.signal.aborted) return;
		if (!(error instanceof ReopenSessionError && error.reason === "mismatch")) {
			showState(status, "blocked", pairingRecovery(error));
			return;
		}
	}
	form.hidden = false;
	let parsed;
	try {
		parsed = await resolveLink(link, (url, init) => fetch(url, {
			...init,
			signal: lifetime.signal
		}));
		if (lifetime.signal.aborted) return;
	} catch {
		if (lifetime.signal.aborted) return;
		form.hidden = true;
		showState(status, "blocked", "This pairing link is unavailable. Run tmt remote pair again for a new link.");
		return;
	}
	form.addEventListener("submit", (event) => {
		event.preventDefault();
		const ownsFocus = form.contains(document.activeElement);
		form.hidden = true;
		if (ownsFocus) status.focus({ preventScroll: true });
		const name = element("name").value.trim();
		ceremony(parsed, name, status, lifetime.signal).catch((error) => {
			if (lifetime.signal.aborted) return;
			if (document.activeElement === element("comparison")) status.focus({ preventScroll: true });
			showState(status, "blocked", error instanceof ReopenSessionError ? pairingRecovery(error) : "Pairing did not complete. Run tmt remote pair again for a new link.");
		});
	}, { signal: lifetime.signal });
	form.hidden = false;
	showState(status, "waiting", "Name this browser and choose Pair to request terminal confirmation.");
	element("state-label").textContent = "Ready to pair";
	button.disabled = false;
}
async function automaticPairingPage(link, lifetime) {
	const status = element("status");
	element("pair").hidden = true;
	element("comparison").hidden = true;
	element("entry-link").hidden = true;
	const timer = setTimeout(() => lifetime.abort(), 6e4);
	let leaving = false;
	const departed = () => {
		leaving = true;
		lifetime.abort();
	};
	globalThis.addEventListener?.("pagehide", departed, { once: true });
	let code;
	let openCommand = "tmt remote open";
	let enrollmentAllowed = false;
	try {
		showState(status, "waiting", "Connecting this browser…");
		const network = (url, init) => fetch(url, {
			...init,
			signal: lifetime.signal
		});
		const offered = await resolveAutomaticLink(link, network);
		code = offered.code;
		const { descriptor, target } = offered;
		if (target === "/colab" || target.startsWith("/colab/") || target.startsWith("/p/") || target.startsWith("/read/")) openCommand = "tmt colab open";
		const record = await existingPairing(lifetime.signal);
		enrollmentAllowed = record === void 0;
		lifetime.signal.throwIfAborted();
		if (record) {
			const key = await DeviceKey.fromHandle(record.handle, record.publicKey);
			await cancelAutomaticOffer({
				descriptor,
				code,
				key,
				kind: "browser",
				origin: location.origin,
				name: "Browser",
				fetch: network,
				signal: lifetime.signal
			});
		} else {
			const key = await DeviceKey.generate();
			const result = await pair({
				descriptor,
				code,
				key,
				kind: "browser",
				origin: location.origin,
				name: "Browser",
				fetch: network,
				deadlineMs: 6e4,
				signal: lifetime.signal
			});
			lifetime.signal.throwIfAborted();
			await save({
				handle: key.handle(),
				publicKey: key.publicKey(),
				paired: result
			});
			enrollmentAllowed = false;
			await openSession(result, key, descriptor.windowId, network);
		}
		lifetime.signal.throwIfAborted();
		location.replace(target);
	} catch (error) {
		if (!leaving && pairingAttempt === lifetime) {
			showState(status, "blocked", "");
			element("state-label").textContent = "Not connected";
			if (error instanceof ReopenSessionError) {
				if (error.reason === "mismatch") {
					const command = document.createElement("code");
					command.className = "tmt-ui-code";
					command.textContent = "tmt remote pair";
					status.replaceChildren(document.createTextNode("This browser is paired with a different machine. To use this one, run "), command, document.createTextNode(" on it."));
				} else status.textContent = pairingRecovery(error);
				return;
			}
			const command = document.createElement("code");
			command.className = "tmt-ui-code";
			command.textContent = openCommand;
			const refused = error instanceof PairingRefusedError;
			const nodes = [
				document.createTextNode(refused ? "This link no longer works. Open the page again from the machine with " : "Could not connect this browser. Open the page again from the machine with "),
				command,
				document.createTextNode(refused || !enrollmentAllowed ? "." : ". If it still fails, run ")
			];
			if (!refused && enrollmentAllowed) {
				const fallback = document.createElement("code");
				fallback.className = "tmt-ui-code";
				fallback.textContent = "tmt remote pair";
				nodes.push(fallback, document.createTextNode(" there."));
			}
			status.replaceChildren(...nodes);
		}
	} finally {
		code?.fill(0);
		clearTimeout(timer);
		globalThis.removeEventListener?.("pagehide", departed);
	}
}
async function ceremony({ descriptor, code }, name, status, signal) {
	const key = await DeviceKey.generate();
	signal.throwIfAborted();
	const indexes = await fingerprintIndexes(key.publicKey());
	const words = element("words");
	words.textContent = indexes.map((i) => WORDS[i]).join(" ");
	const comparison = element("comparison");
	comparison.hidden = false;
	if (document.activeElement === status) comparison.focus({ preventScroll: true });
	showState(status, "waiting", "Waiting for confirmation in your terminal. Compare these words and confirm only if they match.");
	const result = await pair({
		descriptor,
		code,
		key,
		kind: "browser",
		origin: location.origin,
		name,
		signal
	});
	signal.throwIfAborted();
	try {
		await save({
			handle: key.handle(),
			publicKey: key.publicKey(),
			paired: result
		});
	} catch {
		throw new ReopenSessionError("transient", "pairing-unavailable");
	}
	try {
		await openSession(result, key, descriptor.windowId, (url, init) => fetch(url, {
			...init,
			signal
		}));
	} catch {
		throw new ReopenSessionError("unconfirmed", "admission-unconfirmed");
	}
	signal.throwIfAborted();
	showState(status, "paired", "This browser is paired. You can close this page.");
	element("entry-link").hidden = false;
	if (document.activeElement === comparison || document.activeElement === status) element("entry-link").querySelector("a").focus({ preventScroll: true });
}
/** Associate a mounted WebSocket with this tab's verified session, not the shared cookie's tab.
* Use only to construct a transport; never navigate to or log this URL.
*/
function transportUrl(session, value) {
	const channel = channelFor(session);
	if (!channel || channel.ended) throw new RefusalError("REMOTE_SESSION_ENDED");
	const address = new URL(channel.paired.address);
	const url = new URL(value, channel.paired.address);
	if (url.protocol !== (address.protocol === "https:" ? "wss:" : "ws:") || !["http:", "https:"].includes(address.protocol) || url.host !== address.host || !url.pathname.startsWith(`${address.pathname}/x/`) || url.search || url.hash || url.username || url.password) throw new TypeError("Use a mounted WebSocket URL on this door without query or fragment.");
	url.searchParams.set("tmt-session", channel.sessionId);
	return url.href;
}
//#endregion
export { ClientError, RefusalError, RemoteRegistryError, ReopenSessionError, bindSyncContext, budget_exports as budget, certifyKey, firebaseRemote, landingPage, management, operations, pairingPage, remoteRegistry, reopenSession, shellApp, syncDocumentPath, transportUrl, validateSyncDiscovery, validateSyncDocument, validateSyncFields, validateSyncInterest, validateSyncMutation, validateSyncQuery, verifySealedObject };
