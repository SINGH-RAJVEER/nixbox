// Aggregate-only NixBox telemetry receiver for Cloudflare Workers.
// Mirrors crates/nixbox-core/src/telemetry/receiver.rs. It never logs, and never reads or stores client addresses.

import { DurableObject } from "cloudflare:workers";
import { MAX_REPORT_BYTES, parseReport, type Report } from "./schema";

const MAX_DAILY_GROUPS = 4096;
const MAX_TOTAL = 1_000_000_000_000;
const RETAINED_DAYS = 90;
const REQUESTS_PER_MINUTE = 600;
const DAY_MS = 86_400_000;

interface Env {
	TELEMETRY: DurableObjectNamespace<TelemetryStore>;
}

function today(): number {
	return Math.floor(Date.now() / DAY_MS);
}

export class TelemetryStore extends DurableObject<Env> {
	private windowStart = 0;
	private windowCount = 0;

	constructor(ctx: DurableObjectState, env: Env) {
		super(ctx, env);
		ctx.blockConcurrencyWhile(async () => {
			ctx.storage.sql.exec(`CREATE TABLE IF NOT EXISTS daily_counts (
				day INTEGER NOT NULL, app_version TEXT NOT NULL, interface TEXT NOT NULL,
				os TEXT NOT NULL, arch TEXT NOT NULL, observation TEXT NOT NULL, total INTEGER NOT NULL,
				PRIMARY KEY(day, app_version, interface, os, arch, observation)
			) WITHOUT ROWID`);
			this.prune();
			if ((await ctx.storage.getAlarm()) === null) await this.scheduleCleanup();
		});
	}

	private prune(): void {
		this.ctx.storage.sql.exec("DELETE FROM daily_counts WHERE day < ?", today() - (RETAINED_DAYS - 1));
	}

	private async scheduleCleanup(): Promise<void> {
		// Shortly after the next UTC day boundary.
		await this.ctx.storage.setAlarm((today() + 1) * DAY_MS + 30_000);
	}

	async alarm(): Promise<void> {
		this.prune();
		await this.scheduleCleanup();
	}

	/** Returns the HTTP status for the ingestion request. */
	async ingest(report: Report): Promise<number> {
		const now = Date.now();
		if (now - this.windowStart >= 60_000) {
			this.windowStart = now;
			this.windowCount = 0;
		}
		if (this.windowCount >= REQUESTS_PER_MINUTE) return 429;
		this.windowCount += 1;

		const sql = this.ctx.storage.sql;
		const day = today();
		try {
			this.ctx.storage.transactionSync(() => {
				this.prune();
				for (const counter of report.counters) {
					const key = [day, report.app_version, report.interface, report.os, report.arch, counter.observation];
					const exists = sql
						.exec(
							"SELECT 1 FROM daily_counts WHERE day=? AND app_version=? AND interface=? AND os=? AND arch=? AND observation=?",
							...key,
						)
						.toArray().length;
					if (!exists) {
						const groups = sql.exec<{ n: number }>("SELECT COUNT(*) AS n FROM daily_counts WHERE day=?", day).one().n;
						if (groups >= MAX_DAILY_GROUPS) throw new Error("daily counter capacity reached");
					}
					sql.exec(
						`INSERT INTO daily_counts VALUES (?,?,?,?,?,?,?)
						ON CONFLICT(day,app_version,interface,os,arch,observation)
						DO UPDATE SET total=min(?, total+excluded.total)`,
						...key,
						counter.count,
						MAX_TOTAL,
					);
				}
			});
		} catch {
			return 503;
		}
		return 204;
	}
}

function status(code: number): Response {
	return new Response(null, { status: code });
}

function isJson(contentType: string | null): boolean {
	if (contentType === null) return false;
	const mime = contentType.split(";")[0].trim().toLowerCase();
	return mime === "application/json" || (mime.startsWith("application/") && mime.endsWith("+json"));
}

async function readBounded(request: Request): Promise<Uint8Array | null> {
	const declared = request.headers.get("content-length");
	if (declared !== null && Number(declared) > MAX_REPORT_BYTES) return null;
	if (request.body === null) return new Uint8Array();
	const reader = request.body.getReader();
	const chunks: Uint8Array[] = [];
	let size = 0;
	for (;;) {
		const { done, value } = await reader.read();
		if (done) break;
		size += value.byteLength;
		if (size > MAX_REPORT_BYTES) {
			await reader.cancel();
			return null;
		}
		chunks.push(value);
	}
	const body = new Uint8Array(size);
	let offset = 0;
	for (const chunk of chunks) {
		body.set(chunk, offset);
		offset += chunk.byteLength;
	}
	return body;
}

export default {
	async fetch(request: Request, env: Env): Promise<Response> {
		const url = new URL(request.url);
		if (url.protocol === "http:") {
			url.protocol = "https:";
			return Response.redirect(url.toString(), 308);
		}
		if (url.pathname === "/health") {
			return request.method === "GET" || request.method === "HEAD" ? status(204) : status(403);
		}
		if (url.pathname !== "/v1/telemetry") return status(404);
		if (request.method !== "POST") return status(403);
		if (!isJson(request.headers.get("content-type"))) return status(400);

		const body = await readBounded(request);
		if (body === null) return status(413);
		let parsed: unknown;
		try {
			parsed = JSON.parse(new TextDecoder("utf-8", { fatal: true, ignoreBOM: false }).decode(body));
		} catch {
			return status(400);
		}
		const report = parseReport(parsed);
		if (report === null) return status(400);

		try {
			const store = env.TELEMETRY.get(env.TELEMETRY.idFromName("global"));
			return status(await store.ingest(report));
		} catch {
			return status(503);
		}
	},
} satisfies ExportedHandler<Env>;
