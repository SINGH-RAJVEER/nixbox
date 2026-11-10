import { expect, test } from "bun:test";
import { MAX_COUNT, parseReport } from "./schema";

// Output of `nixbox telemetry preview`.
const preview = {
	schema_version: 1,
	app_version: "0.4.0",
	interface: "gui",
	os: "linux",
	arch: "x86-64",
	counters: [
		{ observation: { event: "app_started" }, count: 1 },
		{
			observation: {
				event: "activation_finished",
				target: "nixos",
				activation: "configuration",
				outcome: "succeeded",
				duration: "under5-minutes",
			},
			count: 1,
		},
	],
};

function copy(): typeof preview & Record<string, unknown> {
	return structuredClone(preview);
}

test("accepts the client preview and keys observations like serde", () => {
	const report = parseReport(copy());
	expect(report?.counters.map((counter) => counter.observation)).toEqual([
		'{"event":"app_started"}',
		'{"event":"activation_finished","target":"nixos","activation":"configuration","outcome":"succeeded","duration":"under5-minutes"}',
	]);
});

test("rejects extra fields in every object", () => {
	const targets: ((report: ReturnType<typeof copy>) => Record<string, unknown>)[] = [
		(report) => report,
		(report) => report.counters[0],
		(report) => report.counters[0].observation,
		(report) => report.counters[1].observation,
	];
	for (const target of targets) {
		const report = copy();
		target(report).secret_path = "/home/private/token";
		expect(parseReport(report)).toBeNull();
	}
});

test("rejects versions, counts, enums and duplicate groups", () => {
	for (const version of ["1.0.0+private-host", "1.2", "1.2.-3", "65536.1.1", "00001.0.0"]) {
		expect(parseReport({ ...copy(), app_version: version })).toBeNull();
	}
	for (const count of [0, MAX_COUNT + 1, 1.5]) {
		const report = copy();
		report.counters[0].count = count;
		expect(parseReport(report)).toBeNull();
	}
	expect(parseReport({ ...copy(), schema_version: 2 })).toBeNull();
	expect(parseReport({ ...copy(), arch: "x86_64" })).toBeNull();
	const duplicate = copy();
	duplicate.counters.push(structuredClone(duplicate.counters[0]));
	expect(parseReport(duplicate)).toBeNull();
});
