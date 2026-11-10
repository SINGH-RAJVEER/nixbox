// Mirrors crates/nixbox-core/src/telemetry/schema.rs. Keep both in sync.

export const MAX_REPORT_BYTES = 8 * 1024;
export const MAX_GROUPS = 32;
export const MAX_COUNT = 1000;

const INTERFACES = ["gui", "tui", "cli"];
const OS_FAMILIES = ["linux", "macos", "other"];
const ARCHITECTURES = ["x86-64", "aarch64", "other"];
const TARGETS = ["home-manager", "nixos"];
const ACTIVATIONS = ["configuration", "packages"];
const OUTCOMES = ["succeeded", "failed", "cancelled"];
const DURATIONS = ["under10-seconds", "under1-minute", "under5-minutes", "under15-minutes", "at-least15-minutes"];

export interface Counter {
	// Canonical JSON of the observation, in the same field order serde emits.
	observation: string;
	count: number;
}

export interface Report {
	app_version: string;
	interface: string;
	os: string;
	arch: string;
	counters: Counter[];
}

type Json = Record<string, unknown>;

function isObject(value: unknown): value is Json {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function exactKeys(value: Json, keys: string[]): boolean {
	const actual = Object.keys(value);
	return actual.length === keys.length && keys.every((key) => Object.hasOwn(value, key));
}

function oneOf(value: unknown, allowed: string[]): value is string {
	return typeof value === "string" && allowed.includes(value);
}

function isRelease(value: unknown): value is string {
	if (typeof value !== "string") return false;
	const parts = value.split(".");
	return (
		parts.length === 3 &&
		parts.every((part) => /^[0-9]{1,5}$/.test(part) && Number(part) <= 65535 && String(Number(part)) === part)
	);
}

function observation(value: unknown): string | null {
	if (!isObject(value)) return null;
	if (value.event === "app_started") {
		return exactKeys(value, ["event"]) ? JSON.stringify({ event: "app_started" }) : null;
	}
	if (value.event !== "activation_finished") return null;
	if (!exactKeys(value, ["event", "target", "activation", "outcome", "duration"])) return null;
	if (
		!oneOf(value.target, TARGETS) ||
		!oneOf(value.activation, ACTIVATIONS) ||
		!oneOf(value.outcome, OUTCOMES) ||
		!oneOf(value.duration, DURATIONS)
	) {
		return null;
	}
	return JSON.stringify({
		event: "activation_finished",
		target: value.target,
		activation: value.activation,
		outcome: value.outcome,
		duration: value.duration,
	});
}

/** Returns the validated report, or null for anything the Rust receiver would reject. */
export function parseReport(value: unknown): Report | null {
	if (!isObject(value)) return null;
	if (!exactKeys(value, ["schema_version", "app_version", "interface", "os", "arch", "counters"])) return null;
	if (value.schema_version !== 1) return null;
	if (!isRelease(value.app_version)) return null;
	if (!oneOf(value.interface, INTERFACES) || !oneOf(value.os, OS_FAMILIES) || !oneOf(value.arch, ARCHITECTURES)) {
		return null;
	}
	if (!Array.isArray(value.counters) || value.counters.length === 0 || value.counters.length > MAX_GROUPS) {
		return null;
	}
	const counters: Counter[] = [];
	for (const entry of value.counters) {
		if (!isObject(entry) || !exactKeys(entry, ["observation", "count"])) return null;
		const key = observation(entry.observation);
		const count = entry.count;
		if (key === null || typeof count !== "number" || !Number.isInteger(count) || count < 1 || count > MAX_COUNT) {
			return null;
		}
		if (counters.some((previous) => previous.observation === key)) return null;
		counters.push({ observation: key, count });
	}
	const report: Report = {
		app_version: value.app_version,
		interface: value.interface,
		os: value.os,
		arch: value.arch,
		counters,
	};
	const canonical = JSON.stringify({
		schema_version: 1,
		app_version: report.app_version,
		interface: report.interface,
		os: report.os,
		arch: report.arch,
		counters: counters.map((counter) => ({ observation: JSON.parse(counter.observation), count: counter.count })),
	});
	if (new TextEncoder().encode(canonical).length > MAX_REPORT_BYTES) return null;
	return report;
}
