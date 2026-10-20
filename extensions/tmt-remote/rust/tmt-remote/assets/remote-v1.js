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
function text(value) {
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
function count(value) {
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
	return concat([count(value.length), value]);
}
var lpText = (value) => lp(text(value));
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
	requireValue$1(text(value).length <= 128 && (value === "cli" || addonOrigin(value) || doorOrigin(value)), "origin");
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
	requireValue$1(text(value.operation).length > 0, "operation");
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
	const name = text(value.name);
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
function sendState(value, id) {
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
function row(value, keys) {
	valid$4(typeof value === "object" && value !== null && !Array.isArray(value));
	const r = value;
	valid$4(Object.keys(r).sort().join(",") === keys.sort().join(","));
	return r;
}
function parseSyncMapping(value) {
	valid$4(typeof value === "object" && value !== null);
	const r = row(value, [
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
		const q = row(r.query, [
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
			const d = row(q.discovery, [
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
	const root = new URL("./", pageUrl);
	if (root.pathname === "/remote/") root.pathname = "/";
	return entry.kind === "local" && (entry.url === address || new URL(entry.url).href === root.href);
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
function field(label, value) {
	const wrapper = node("label", label);
	wrapper.className = "tmt-ui-field-label";
	const input = node("input");
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
		const box = node("dialog");
		box.className = "remote-dialog";
		const heading = node("h2", title);
		heading.id = "remote-dialog-heading";
		box.setAttribute("aria-labelledby", heading.id);
		const body = node("div");
		const message = node("p");
		message.setAttribute("role", "status");
		const buttons = node("div");
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
		view.body.append(node("p", "Only names and addresses are copied. No keys, pairings or drafts."));
		const choices = [];
		for (const row of plan.rows) {
			const item = node("section");
			item.className = "remote-import-row";
			item.append(node("h3", row.entry.name), node("p", `${kindLabel(row.entry)} · ${row.entry.url}`));
			if (row.status === "duplicate") {
				item.append(node("p", `Already added as ${row.existingName}`));
				choices.push(() => null);
			} else if (row.status === "collision") {
				item.append(node("p", `You already have a remote named ${row.entry.name}`));
				const alias = field("Remote name", row.suggestedName);
				const skip = node("input");
				skip.type = "checkbox";
				const skipLabel = node("label", "Skip");
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
				row.dataset.remote = entry.name;
				row.dataset.url = entry.url;
				row.dataset.kind = entry.kind;
				row.append(node("h3", entry.name), node("p", `${kindLabel(entry)} · ${entry.url}`));
				const state = node("p", remoteState(observed, current(entry)).text);
				state.className = "remote-state";
				state.dataset.tone = remoteState(observed, current(entry)).tone;
				row.append(state);
				const actions = node("div");
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
			if (!rows.length) list.append(node("li", "No remotes added in this browser."));
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
	if (error instanceof ReopenSessionError && error.reason === "capacity") return "This browser has reached the owner's connection limit. Close another tab, or ask the owner to change the limit. Your pairing is unchanged.";
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
async function landingPage() {
	if (document.readyState === "loading") await new Promise((resolve) => document.addEventListener("DOMContentLoaded", () => resolve(), { once: true }));
	const registryState = remoteRegistryPage();
	const button = element("check");
	let attempt = 0;
	let departed = false;
	let session;
	let opaque = false;
	const pageAlive = () => !departed && button.isConnected;
	window.addEventListener("pagehide", () => {
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
				"○",
				"Not paired",
				"Pair this browser with your machine",
				""
			],
			checking: [
				"◌",
				"Checking",
				"Checking the connection…",
				"This browser is paired. Nothing is sent while checking."
			],
			connected: [
				"●",
				"Connected",
				"This browser can use Remote",
				"Open your app from its link in this browser."
			],
			different: [
				"△",
				"Different machine",
				"This browser is paired with another machine",
				"This Remote runs on a machine this browser isn't paired with. Your existing pairing is kept."
			],
			refused: [
				"×",
				"Not accepted",
				"Remote didn't accept this browser",
				refusalCode === "REMOTE_SESSION_LIMIT" ? "This browser has reached the owner's connection limit. Close another tab, or ask the owner to change the limit. Your pairing is unchanged." : authorityEnded ? "This pairing was removed or has expired. To use Remote here, run tmt remote pair on the machine." : "This page's connection ended. Try again. Your pairing is unchanged."
			],
			unconfirmed: [
				"×",
				"Can't reach Remote",
				"Couldn't confirm the connection",
				"Remote may have stopped, or its reply couldn't be verified. Your pairing is unchanged."
			],
			unreadable: [
				"×",
				"Pairing unreadable",
				"This browser's pairing can't be read",
				"Your saved pairing could not be read. Check browser storage and try again."
			]
		}[state];
		element("notice").dataset.tone = state === "connected" ? "working" : [
			"missing",
			"checking",
			"different"
		].includes(state) ? "waiting" : "blocked";
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
		if (!lifetime.signal.aborted) showState(status, "blocked", pairingRecovery(error));
		return;
	}
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
				status.textContent = pairingRecovery(error);
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
export { ClientError, RefusalError, RemoteRegistryError, ReopenSessionError, bindSyncContext, budget_exports as budget, certifyKey, firebaseRemote, landingPage, management, operations, pairingPage, remoteRegistry, reopenSession, syncDocumentPath, transportUrl, validateSyncDiscovery, validateSyncDocument, validateSyncFields, validateSyncInterest, validateSyncMutation, validateSyncQuery, verifySealedObject };
