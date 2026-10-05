//#region src/canonical-bytes.ts
var encoder = new TextEncoder();
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
	return encoder.encode(value);
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
	constructor(code, retryAfterMs, limit, settingsUrl) {
		super(`Remote operation refused: ${code}.`);
		this.code = code;
		this.retryAfterMs = retryAfterMs;
		this.limit = limit;
		this.settingsUrl = settingsUrl;
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
	"REMOTE_MANAGEMENT_CAPACITY"
]);
var UUID$2 = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
var V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
var coreId = (value) => typeof value === "string" && UUID$2.test(value) && value !== "00000000-0000-0000-0000-000000000000";
var requestId = (value) => typeof value === "string" && value.startsWith("req_") && coreId(value.slice(4));
var utf8$1 = new TextEncoder();
var strictUtf8$1 = new TextDecoder("utf-8", { fatal: true });
var MAX_SEQUENCE = 18446744073709551615n;
function valid$1(condition) {
	if (!condition) throw new Error("Invalid operation response.");
}
function input$1(condition) {
	if (!condition) throw new TypeError("Invalid Remote operation input.");
}
function record(value) {
	valid$1(typeof value === "object" && value !== null && !Array.isArray(value));
	return value;
}
function reason(value) {
	if (value.reason === void 0) return {};
	valid$1(typeof value.reason === "string" && utf8$1.encode(value.reason).length <= 256);
	return { reason: value.reason };
}
function sendState(value, id) {
	const state = record(value);
	valid$1(state.operationId === id);
	switch (state.state) {
		case "held": return {
			state: "held",
			operationId: id
		};
		case "accepted":
			valid$1(requestId(state.requestId));
			return {
				state: "accepted",
				operationId: id,
				requestId: state.requestId
			};
		case "uncertain":
			valid$1(state.requestId === void 0 || requestId(state.requestId));
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
	valid$1(state.requestId === id);
	switch (state.state) {
		case "pending": return {
			state: "pending",
			requestId: id
		};
		case "replied":
			valid$1(typeof state.message === "string");
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
	valid$1(Array.isArray(rows));
	return rows.map((value) => {
		const row = record(value);
		valid$1(coreId(row.id) && typeof row.name === "string" && (row.presence === "active" || row.presence === "offline" || row.presence === "unknown"));
		return {
			id: row.id,
			name: row.name,
			presence: row.presence,
			...Object.hasOwn(row, "delivery") ? { delivery: row.delivery } : {}
		};
	});
}
function remoteError(value, address) {
	if (!Object.hasOwn(value, "error")) return void 0;
	const error = record(value.error);
	valid$1(typeof error.code === "string" && /^[A-Z][A-Z0-9_]{0,63}$/.test(error.code) && typeof error.message === "string");
	const limit = error.limit;
	valid$1(limit === void 0 || typeof limit === "number" && Number.isSafeInteger(limit) && limit > 0);
	valid$1(error.code !== "REMOTE_SESSION_EVICTED" || limit !== void 0);
	let settingsUrl;
	if (error.settingsUrl !== void 0 && error.settingsUrl !== null) {
		valid$1(typeof error.settingsUrl === "string" && error.settingsUrl.length > 0 && error.settingsUrl.length <= 2048);
		const url = new URL(error.settingsUrl, address);
		const door = new URL(address);
		valid$1(url.origin === door.origin && !url.username && !url.password && ["http:", "https:"].includes(url.protocol));
		settingsUrl = url.href;
	}
	const retry = error.retryAfterMs;
	valid$1(retry === void 0 || typeof retry === "number" && Number.isSafeInteger(retry) && retry >= 0 && retry <= 6e4);
	if (error.code === "REMOTE_REPLAY") return new SequenceMismatch();
	valid$1(PRE_EFFECT.has(error.code) || [
		"REMOTE_INPUT_TOO_LARGE",
		"REMOTE_STATE_UNAVAILABLE",
		"REMOTE_CORE_UNAVAILABLE",
		"REMOTE_MANAGEMENT_READ_ONLY",
		"REMOTE_MANAGEMENT_UNAVAILABLE",
		"REMOTE_DEVICE_REVOKED",
		"REMOTE_DEVICE_NOT_FOUND",
		"REMOTE_SETTINGS_UNAVAILABLE",
		"REMOTE_MANAGEMENT_CAPACITY"
	].includes(error.code));
	return new RefusalError(error.code, retry, limit, settingsUrl);
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
/** Scope-free reads synchronize the lane; the caller's operation is never a probe. */
async function synchronize(channel, timeoutMs) {
	const unresolved = channel.uncertainSequence;
	if (unresolved === void 0) return;
	const unavailable = () => new ClientError("sequence_unavailable", "Remote sequence recovery failed; reopen before observing the original operation.");
	if (unresolved === "unavailable") throw unavailable();
	const id = crypto.randomUUID();
	const payload = utf8$1.encode("{}");
	function capabilities(value) {
		const reply = record(value);
		valid$1(reply.version === 1 && reply.profile === "local-v1" && reply.binding === "loopback-http");
		valid$1(Array.isArray(reply.operations) && reply.operations.every((value) => typeof value === "string"));
		record(reply.limits);
	}
	try {
		try {
			await attempt(channel, timeoutMs, "capabilities", id, payload, channel.clientSequence, capabilities);
		} catch (error) {
			if (!(error instanceof SequenceMismatch)) throw error;
			await attempt(channel, timeoutMs, "capabilities", id, payload, unresolved, capabilities);
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
					...error.settingsUrl === void 0 ? {} : { settingsUrl: error.settingsUrl }
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
					...error.settingsUrl === void 0 ? {} : { settingsUrl: error.settingsUrl }
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
		...activity ? [
			"thisBrowser",
			"liveSessionCount",
			"lastActivityAtMs"
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
	valid(integer(row.revision) && row.revision > 0 && typeof row.revoked === "boolean");
	if (activity) valid(typeof row.thisBrowser === "boolean" && integer(row.liveSessionCount) && (row.lastActivityAtMs === null || integer(row.lastActivityAtMs)));
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
	"REMOTE_MANAGEMENT_CAPACITY"
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
	async function mutate(operation, value) {
		const { operationId } = value;
		input(UUID$1.test(operationId));
		try {
			return await call(operation, operationId, value, (reply) => {
				const parsed = outcome(reply, operationId);
				if (parsed.state === "committed") {
					if (operation === "remote.settings.set") valid("settings" in parsed.result);
					else {
						valid("device" in parsed.result && parsed.result.device.clientId === value.clientId);
						if (operation === "remote.devices.rename") valid(parsed.result.device.name === value.name);
						if (operation === "remote.devices.revoke") valid(parsed.result.device.revoked);
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
		settings: () => call("remote.settings.show", crypto.randomUUID(), {}, (value) => {
			const row = object(value);
			exact(row, [
				"settings",
				"capabilities",
				"readOnlyReason"
			]);
			settings(row.settings);
			const capabilities = object(row.capabilities);
			exact(capabilities, ["settingsWrite", "devicesWrite"]);
			valid(typeof capabilities.settingsWrite === "boolean" && typeof capabilities.devicesWrite === "boolean");
			valid(row.readOnlyReason === null || row.readOnlyReason === "local_cli_required");
			valid(capabilities.settingsWrite === capabilities.devicesWrite && capabilities.settingsWrite === (row.readOnlyReason === null));
			return row;
		}),
		devices: (page) => {
			input(Number.isInteger(page.limit) && page.limit >= 1 && page.limit <= 50 && (page.cursor === null || typeof page.cursor === "string" && /^[A-Za-z0-9_-]{98}$/.test(page.cursor)));
			return call("remote.devices.list", crypto.randomUUID(), page, (value) => {
				const row = object(value);
				exact(row, ["devices", "nextCursor"]);
				valid(Array.isArray(row.devices) && row.devices.length <= page.limit);
				const ids = /* @__PURE__ */ new Set();
				for (const entry of row.devices) {
					const item = device(entry, true);
					valid(!ids.has(item.clientId));
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
	requireValue(url.protocol === "http:" && url.hostname === "127.0.0.1" && url.pathname === "/pair" && url.search === "" && url.username === "" && url.password === "" && /^[A-Z2-7]{26}$/.test(url.hash.slice(1)), "pairing link");
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
/**
* Submit one enrollment candidate and retry it exactly while the owner has
* not answered. The machine key and grant are accepted only after
* `serverProof` verifies over the exact receipt bytes.
*/
async function pair(options) {
	const { descriptor, code, key } = options;
	const send = options.fetch ?? fetch;
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
	const signature = await key.sign(enrollmentPossessionSigningBytes(enrollment, mac));
	const body = JSON.stringify({
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
		signature: base64url(signature)
	});
	const deadline = Date.now() + (options.deadlineMs ?? 6e5);
	for (;;) {
		const response = await send(`${descriptor.address}/pair`, {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body
		});
		if (response.status === 202 && Date.now() < deadline) continue;
		if (response.status !== 200) throw new Error("Pairing was refused or ended.");
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
async function openSession(paired, key, windowId, send = fetch) {
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
	const session = JSON.parse(strictUtf8.decode(reply.payload));
	requireValue(session.sessionId === reply.sessionId && Number.isSafeInteger(session.serverTimeMs) && session.serverTimeMs >= 0 && Number.isSafeInteger(session.grantRevision) && session.grantRevision > 0 && (session.expiresAtMs === null || Number.isSafeInteger(session.expiresAtMs) && session.expiresAtMs >= 0), "session payload");
	registerChannel(session, paired, key, windowId, send);
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
/**
* Browser entry of the device SDK, served by the door as `/sdk/remote-v1.js`.
* On the pairing page it runs the ceremony; mounted extension pages import it
* to reopen the door session and certify their own extension keys. The device
* key lives in this origin's IndexedDB as an opaque, non-extractable CryptoKey.
*/
var WORDS = bip39_english_default.split("\n").slice(0, 2048);
var DATABASE = "tmt-remote";
var STORE = "device";
var RECORD = "device";
function request(open) {
	return new Promise((resolve, reject) => {
		const pending = open();
		pending.onsuccess = () => resolve(pending.result);
		pending.onerror = () => reject(pending.error ?? /* @__PURE__ */ new Error("Device storage failed."));
	});
}
async function store(mode) {
	const opening = indexedDB.open(DATABASE, 1);
	opening.onupgradeneeded = () => opening.result.createObjectStore(STORE);
	return (await request(() => opening)).transaction(STORE, mode).objectStore(STORE);
}
async function load() {
	const objects = await store("readonly");
	return await request(() => objects.get(RECORD));
}
async function save(record) {
	const objects = await store("readwrite");
	await request(() => objects.put(record, RECORD));
}
async function door() {
	const response = await fetch("/sdk/mount", {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ path: location.pathname })
	});
	if (response.status !== 200) throw new Error("The door did not answer.");
	return await response.json();
}
async function paired() {
	const record = await load();
	if (!record) throw new Error("This browser is not paired.");
	return {
		record,
		key: await DeviceKey.fromHandle(record.handle, record.publicKey)
	};
}
/** Reopen this browser's door session for the running remote; no owner step. */
async function reopenSession() {
	const { record, key } = await paired();
	const current = await door();
	if (current.machineId !== record.paired.machineId) throw new Error("Paired with another machine.");
	if (!current.address.startsWith(`${location.origin}/r/`) || !/^[a-z2-7]{16}$/.test(current.address.slice(`${location.origin}/r/`.length))) throw new Error("The door advertised an invalid address.");
	return openSession({
		...record.paired,
		address: current.address
	}, key, current.windowId);
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
	if (!found) throw new Error(`The pairing page lacks #${id}.`);
	return found;
}
/** Present the ceremony state without coloring its instruction text. */
function showState(status, state, text) {
	status.dataset.state = state;
	status.textContent = text;
	element("mark").textContent = {
		waiting: "◆",
		paired: "✓",
		blocked: "✗"
	}[state];
}
/** Called by the fragment-erasing bootstrap after the page DOM is ready. */
async function pairingPage(link) {
	if (document.readyState === "loading") await new Promise((resolve) => document.addEventListener("DOMContentLoaded", () => resolve(), { once: true }));
	const status = element("status");
	const form = element("pair");
	const button = form.querySelector("button");
	button.disabled = true;
	let parsed;
	try {
		parsed = await resolveLink(link);
	} catch {
		form.hidden = true;
		showState(status, "blocked", "This pairing link is unavailable. Run tmt remote pair again for a new link.");
		return;
	}
	button.disabled = false;
	form.addEventListener("submit", (event) => {
		event.preventDefault();
		form.hidden = true;
		const name = element("name").value.trim();
		ceremony(parsed, name, status).catch(() => {
			showState(status, "blocked", "Pairing did not complete. Run tmt remote pair again for a new link.");
		});
	});
}
async function ceremony({ descriptor, code }, name, status) {
	const key = await DeviceKey.generate();
	const indexes = await fingerprintIndexes(key.publicKey());
	const words = element("words");
	words.textContent = indexes.map((i) => WORDS[i]).join(" ");
	element("comparison").hidden = false;
	showState(status, "waiting", "Compare these words with the terminal, then confirm there.");
	const result = await pair({
		descriptor,
		code,
		key,
		kind: "browser",
		origin: location.origin,
		name
	});
	await save({
		handle: key.handle(),
		publicKey: key.publicKey(),
		paired: result
	});
	await openSession(result, key, descriptor.windowId);
	showState(status, "paired", "This browser is paired. You can close this page.");
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
export { ClientError, RefusalError, certifyKey, management, operations, pairingPage, reopenSession, transportUrl };
