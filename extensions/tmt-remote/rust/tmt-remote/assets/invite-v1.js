//#endregion
//#region src/service-commands.ts
/** Copy-only terminal references. Native grammar tests admit the same product catalog. */
var serviceCommands = Object.freeze({
	status: "tmt remote status --layers",
	preview: "tmt remote deploy firestore --project='<project-id>' --region='<region>' --defaults --json",
	deploy: "tmt remote deploy firestore --project='<project-id>' --region='<region>' --defaults",
	nodeVersion: "node --version",
	firebaseVersion: "firebase --version",
	firebaseInstall: "npm install -g firebase-tools@15.29.0",
	firebaseLogin: "firebase login",
	join: "tmt remote join <code>"
});
//#endregion
//#region src/invite-page.ts
/** Instructions only. Parsing is not authentication; native join owns admission. */
var MAX_BYTES = 12288;
var MAX_INVITATION_INPUT = Math.ceil(MAX_BYTES * 4 / 3) + 2048;
var UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
function valid(condition) {
	if (!condition) throw new Error("This invitation cannot be read. Ask its owner for a new code.");
}
function object(value, keys) {
	valid(value !== null && typeof value === "object" && !Array.isArray(value));
	const row = value;
	valid(Object.keys(row).length === keys.length && keys.every((key) => Object.hasOwn(row, key)));
	return row;
}
function bytes(value, length) {
	valid(Array.isArray(value) && value.length > 0 && value.length <= MAX_BYTES);
	valid(length === void 0 || value.length === length);
	valid(value.every((byte) => Number.isInteger(byte) && byte >= 0 && byte <= 255));
	return new Uint8Array(value);
}
function hasControl(value) {
	return Array.from(value).some((char) => {
		const point = char.codePointAt(0);
		return point < 32 || point >= 127 && point <= 159;
	});
}
var utf8 = new TextDecoder("utf-8", { fatal: true });
/** Reads the native v2 carrier unchanged, with no network or authority inference. */
function decodeDetails(token, now) {
	valid(token.length > 0 && token.length <= Math.ceil(MAX_BYTES * 4 / 3));
	valid(/^[A-Za-z0-9_-]+$/.test(token));
	const encoded = token.replace(/-/g, "+").replace(/_/g, "/");
	const raw = atob(encoded);
	valid(raw.length <= MAX_BYTES && btoa(raw).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "") === token);
	const text = utf8.decode(Uint8Array.from(raw, (char) => char.charCodeAt(0)));
	const value = JSON.parse(text);
	valid(JSON.stringify(value) === text);
	const invite = object(value, [
		"version",
		"context",
		"publicConfig",
		"machineName",
		"code"
	]);
	valid(invite.version === 2);
	bytes(invite.code, 16);
	valid(typeof invite.machineName === "string" && invite.machineName.length > 0 && new TextEncoder().encode(invite.machineName).length <= 64 && !hasControl(invite.machineName) && invite.machineName.replace(/\uFEFF/g, "").trim().length > 0);
	const context = object(invite.context, [
		"binding",
		"owner",
		"owner_key",
		"offer",
		"nonce",
		"issued_at_ms",
		"expires_at_ms"
	]);
	valid(typeof context.owner === "string" && UUID.test(context.owner));
	valid(typeof context.offer === "string" && UUID.test(context.offer));
	bytes(context.owner_key, 32);
	bytes(context.nonce, 16);
	const issued = context.issued_at_ms;
	const expiry = context.expires_at_ms;
	valid(typeof issued === "number" && Number.isSafeInteger(issued) && issued >= 0);
	valid(typeof expiry === "number" && Number.isSafeInteger(expiry) && expiry > issued && expiry - issued <= 6e5);
	valid(issued <= now + 6e4 && now < expiry);
	const binding = object(context.binding, [
		"backend",
		"reference",
		"owner_principal"
	]);
	valid(binding.backend === "firestore");
	const project = utf8.decode(bytes(binding.reference));
	const principal = utf8.decode(bytes(binding.owner_principal));
	valid(principal.length > 0 && new TextEncoder().encode(principal).length <= 128 && principal !== "." && principal !== ".." && !hasControl(principal) && !principal.includes("/"));
	const config = invite.publicConfig;
	valid(config !== null && typeof config === "object" && !Array.isArray(config));
	valid(typeof project === "string" && /^[a-z][a-z0-9-]{4,28}[a-z0-9]$/.test(project) && config.projectId === project);
	valid(typeof config.apiKey === "string" && /^[A-Za-z0-9_-]{1,256}$/.test(config.apiKey));
	valid(typeof config.appId === "string" && /^1:[0-9]{1,32}:web:.+$/.test(config.appId));
	valid(config.authDomain === void 0 || config.authDomain === `${project}.firebaseapp.com`);
	return {
		owner: context.owner,
		offer: context.offer,
		expiry
	};
}
/** Public carrier facts only; these do not authenticate the inviter or grant access. */
function invitationDetails(token, now = Date.now()) {
	try {
		return decodeDetails(token, now);
	} catch {
		throw new Error("This invitation cannot be read. Ask its owner for a new code.");
	}
}
function invitationExpiry(token, now = Date.now()) {
	try {
		return invitationDetails(token, now).expiry;
	} catch {
		throw new Error("This invitation cannot be read. Ask its owner for a new code.");
	}
}
/** Accept a native code or its hosted /join link without fetching or trusting the origin. */
function invitationFromInput(input, now = Date.now()) {
	try {
		valid(input.length <= MAX_INVITATION_INPUT);
		const value = input.trim();
		let token = value;
		if (value.includes("://")) {
			const url = new URL(value);
			valid((url.protocol === "https:" || url.protocol === "http:") && !url.username && !url.password && url.pathname === "/join" && !url.search && url.hash.length > 1);
			token = url.hash.slice(1);
		}
		return {
			token,
			expiry: invitationExpiry(token, now)
		};
	} catch {
		throw new Error("This invitation cannot be read. Ask its owner for a new code.");
	}
}
/** Copy the validated native command; this never runs a command or reads the provider. */
async function copyInvitation(token) {
	invitationExpiry(token);
	await navigator.clipboard.writeText(serviceCommands.join.replace("<code>", token));
}
/** Owns one memory-only interaction; the bootstrap erases the fragment first. */
function invitePage(initialToken) {
	const button = document.getElementById("invite-copy");
	const status = document.getElementById("invite-status");
	const feedback = document.getElementById("invite-feedback");
	let token = initialToken;
	initialToken = "";
	let disposed = false;
	let timer;
	const clear = () => {
		disposed = true;
		token = "";
		button.disabled = true;
		feedback.textContent = "";
		if (timer !== void 0) clearTimeout(timer);
		button.removeEventListener("click", copy);
	};
	const copy = async () => {
		if (disposed) return;
		try {
			await copyInvitation(token);
			if (!disposed) feedback.textContent = "Command copied. Run it in your terminal.";
		} catch {
			if (!disposed) feedback.textContent = "Could not copy. Use the original code in your terminal, or ask your inviter for a new one.";
		}
	};
	try {
		const expiry = invitationExpiry(token);
		status.textContent = "Copy the command to continue in your terminal. This page has not checked whether the invitation is still available.";
		button.disabled = false;
		button.addEventListener("click", copy);
		timer = setTimeout(() => {
			clear();
			status.textContent = "This invitation has reached its expiry. Ask your inviter for a new one. This page cannot confirm cloud cleanup.";
		}, Math.max(0, expiry - Date.now()));
	} catch {
		clear();
		status.textContent = "This invitation is missing, invalid or expired. Use the original code in your terminal, or ask your inviter for a new one.";
	}
	return clear;
}
//#endregion
export { MAX_INVITATION_INPUT, copyInvitation, invitationDetails, invitationExpiry, invitationFromInput, invitePage };
