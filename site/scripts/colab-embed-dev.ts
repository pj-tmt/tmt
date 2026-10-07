import { execFile } from "node:child_process";
import type { Plugin } from "vite-plus";

export const embedPrefix = "/__tmt_embed/";
export function loopbackPeer(address: string | undefined): boolean {
  if (address === "::1") return true;
  const ipv4 = address?.replace(/^::ffff:/, "");
  return (
    !!ipv4 &&
    /^127\.(?:\d{1,3}\.){2}\d{1,3}$/.test(ipv4) &&
    ipv4.split(".").every((part) => Number(part) <= 255)
  );
}
export function allowedRequest(host: string | undefined, origin: string | undefined): boolean {
  if (!host || !/^(127\.0\.0\.1|localhost):[0-9]+$/.test(host)) return false;
  return !origin || origin === `http://${host}`;
}
function cli(args: string[]): Promise<unknown> {
  return new Promise((resolve, reject) => {
    execFile(
      "tmt",
      [...args, "--json"],
      { timeout: 12_000, maxBuffer: 512 * 1024 },
      (error, stdout) => {
        try {
          const value: unknown = JSON.parse(stdout);
          if (error && !(value && typeof value === "object" && "status" in value))
            reject(
              new Error(
                value &&
                  typeof value === "object" &&
                  "error" in value &&
                  value.error &&
                  typeof value.error === "object" &&
                  "message" in value.error &&
                  typeof value.error.message === "string"
                  ? value.error.message
                  : "TMT command failed. Check the local terminal.",
              ),
            );
          else resolve(value);
        } catch {
          reject(new Error(error?.message ?? "TMT returned an invalid response"));
        }
      },
    );
  });
}
/** Local developer access, not Remote device authority. Never included in a built site. */
export function colabEmbedDev(run: (args: string[]) => Promise<unknown> = cli): Plugin {
  const requests = new Set<string>();
  const submissions = new Map<
    string,
    { agent: string; message: string; sender: string; result: Promise<unknown> }
  >();
  return {
    name: "tmt-colab-embed-dev",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use(async (req, res, next) => {
        const pathname = req.url?.split("?")[0];
        if (!pathname?.startsWith(embedPrefix)) return next();
        res.setHeader("Content-Type", "application/json");
        res.setHeader("Cache-Control", "no-store");
        const respond = (code: number, value: unknown) => {
          res.statusCode = code;
          res.end(JSON.stringify(value));
        };
        if (
          !loopbackPeer(req.socket.remoteAddress) ||
          !allowedRequest(req.headers.host, req.headers.origin) ||
          req.headers["x-tmt-embed"] !== "1"
        )
          return respond(403, { error: "Local handbook origin required" });
        try {
          if (req.method === "GET" && pathname === `${embedPrefix}status`) {
            const [agents, remote] = await Promise.all([
              run(["ls"]),
              run(["remote", "status"]).catch(() => null),
            ]);
            return respond(200, { agents, remote });
          }
          if (req.method === "GET" && pathname.startsWith(`${embedPrefix}result/`)) {
            const id = pathname.slice(`${embedPrefix}result/`.length);
            if (!requests.has(id))
              return respond(404, { error: "Request is not owned by this preview session" });
            return respond(200, await run(["result", id]));
          }
          if (req.method !== "POST" || pathname !== `${embedPrefix}send`)
            return respond(405, { error: "Unsupported operation" });
          if (!req.headers["content-type"]?.startsWith("application/json"))
            return respond(415, { error: "JSON required" });
          req.setEncoding("utf8");
          let body = "";
          for await (const chunk of req) {
            body += String(chunk);
            if (Buffer.byteLength(body) > 24_000)
              return respond(413, { error: "Message too large" });
          }
          const value: unknown = JSON.parse(body);
          if (!value || typeof value !== "object")
            return respond(400, { error: "Invalid message" });
          const { agent, message, operationId } = value as Record<string, unknown>;
          if (
            typeof operationId !== "string" ||
            !/^[0-9a-f-]{36}$/.test(operationId) ||
            typeof agent !== "string" ||
            !/^[0-9a-f-]{36}$/.test(agent) ||
            typeof message !== "string" ||
            !message.trim() ||
            message.length > 16_000
          )
            return respond(400, { error: "Choose an agent and enter a message" });
          const sender = process.env.TMT_EMBED_IDENTITY;
          if (!sender)
            return respond(503, { error: "Set TMT_EMBED_IDENTITY in the preview terminal" });
          const previous = submissions.get(operationId);
          if (
            previous &&
            (previous.agent !== agent || previous.message !== message || previous.sender !== sender)
          )
            return respond(409, { error: "Send operation already belongs to a different message" });
          if (!previous && submissions.size >= 256)
            return respond(429, {
              error: "Preview send limit reached. Finish pending requests before restarting.",
            });
          const operation = previous ?? {
            agent,
            message,
            sender,
            result: Promise.resolve().then(async () => {
              const roster = (await run(["ls"])) as { identities: { id: string; name: string }[] };
              const target = roster.identities.find((item) => item.id === agent);
              if (!target) throw new Error("Agent is no longer available. Reconnect to refresh.");
              const result = await run([
                "talk",
                target.name,
                message,
                "--detach",
                "--identity",
                sender,
              ]);
              if (
                result &&
                typeof result === "object" &&
                "requestId" in result &&
                typeof result.requestId === "string"
              )
                requests.add(result.requestId);
              return result;
            }),
          };
          // Cache before dispatch starts. An uncertain failure keeps its operation;
          // retrying that operation never executes talk a second time.
          if (!previous) submissions.set(operationId, operation);
          const result = await operation.result;
          return respond(200, result);
        } catch (error) {
          respond(502, { error: error instanceof Error ? error.message : "Connection failed" });
        }
      });
    },
  };
}
