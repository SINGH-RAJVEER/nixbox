# Optional telemetry implementation plan

Status: consent persistence, the typed report schema, bounded collector, GUI/TUI/CLI controls, activation instrumentation, and Docker/Nginx hosting are implemented. The receiver is deployed and verified at `https://nixb.duckdns.org:58443/v1/telemetry`. All new GUI/TUI/CLI builds share that recipient. Collection remains off by default and requires explicit consent. Automated tests inject controlled receivers; production verification uses synthetic reports sent explicitly by the maintainer.

The shared telemetry module covers release adoption, interface usage, supported platforms, and aggregate activation outcomes. Collection remains off until explicit consent and a configured recipient both permit it. These measurements should help choose supported builds and spot changes in reported rebuild reliability.

## Collection contract

- Fresh installs and existing settings files without telemetry fields default to disabled. Upgrading never enables collection.
- While disabled, create no telemetry worker, HTTP client, counters, queue, identifier, or telemetry files. Perform no telemetry DNS lookup or request. Ordinary Nix and GitHub functionality keeps its existing network behavior.
- Collect only actions occurring during a valid consent period. Enabling collection never imports search history, manifests, recovery state, or earlier activity.
- Disabling stops collection in the current process, discards pending reports, and cancels its active request. Other instances recheck saved consent before aggregating or sending. A request already transmitted cannot be recalled.
- Telemetry failures never change command results, rebuild outcomes, terminal output, or UI responsiveness. Delivery is optional and may lose reports.
- Show the data categories and destination before enabling. Provide an offline payload example. Use a normal settings row, with no startup prompt, banner, recurring reminder, or reward for enabling it.

Use the wording "basic usage data" rather than promising anonymity. A receiving server necessarily sees the connection's source IP, even when the payload contains no identifier. The design follows the data minimization and consent principles in the [W3C Privacy Principles](https://www.w3.org/TR/privacy-principles/).

## Allowed data

Use two typed observations. Aggregate their counts in memory before delivery and store only aggregate counters at the receiver.

| Observation | Allowed fields | Counting rule |
| --- | --- | --- |
| `app_started` | Schema version, app release, interface, OS family, CPU architecture | Once after successful GUI/TUI initialization, or dispatch of an eligible CLI command. |
| `activation_finished` | The same common fields, target, activation kind, outcome, duration bucket | Once per completed activation attempt, including a retry as a separate attempt. |

Constrain every dimension. Interface is `gui`, `tui`, or `cli`, regardless of CLI alias. OS and architecture use small allowlists with `other`. Release is the Cargo major/minor/patch version without custom build metadata. Target is `home-manager` or `nixos`. Activation kind is `configuration` or `packages`. Outcome is `succeeded`, `failed`, or `cancelled`. Duration uses monotonic elapsed time, bucketed into under 10 seconds, 10 to 59 seconds, 1 to 4 minutes, 5 to 14 minutes, and 15 minutes or longer.

Do not collect search queries, package names, installed package lists, flake URLs or revisions, option paths or values, file paths, usernames, hostnames, machine identifiers, credentials, command arguments, environment contents, raw errors, build logs, panic reports, hardware inventories, or precise client timestamps. Do not hash these values and send the hashes. Do not create installation IDs, session IDs, cookies, or device fingerprints. UI clicks and page visits are outside this first version.

An activation attempt covers the whole profile update or configuration rebuild. Internal subprocesses and profile follow-up steps do not each count as attempts. A failed attempt means the runner reported failure; it does not prove that files or the active system stayed unchanged. Dry runs, declined confirmations, writes with `--no-rebuild`, and profile generation/rollback commands do not produce activation observations. Missing terminal events and process crashes remain unreported.

Without persistent identifiers, launch counts cannot measure unique installations, active users, or retention. Opt-in participation and delivery losses also bias the results. Report outcomes as counts of observed completed attempts, with explicit denominators. Do not present them as overall user success rates.

## Consent and settings

The serde-defaulted `TelemetrySettings` value in `nixbox-config::Config` now stores consent, with the types and policy in `crates/nixbox-config/src/telemetry.rs`:

```json
{
	"telemetry": {
		"enabled": false,
		"consent_version": 0,
		"revision": 0
	}
}
```

`consent_version` records the disclosed collection contract. Version 3 discloses the deployed recipient `https://nixb.duckdns.org:58443/v1/telemetry`, including its forwarded HTTPS port. Consent from versions 1 and 2 is insufficient. Enabling through a supported control sets the current version. New categories or a new recipient require fresh consent; ordinary releases using the same contract retain the existing choice. `revision` is a local counter changed by explicit consent updates. It never leaves the machine. Pending observations from a different revision must be discarded, including after a rapid disable/enable cycle.

Effective collection requires saved enablement, the current consent version, and no disabling override. Support `NIXBOX_NO_TELEMETRY=1` and `DO_NOT_TRACK=1`. Environment variables may disable collection but may never enable it. Suppress production delivery in debug builds, automated tests, CI, and the existing `NIXBOX_VM_BUILD` workflow. Tests use an explicitly injected local receiver.

`Config::save` now serializes settings updates with a shared lock, writes a synchronized temporary file, and replaces the destination atomically. Ordinary saves preserve the latest telemetry choice from disk. `Config::set_telemetry_enabled` persists explicit consent changes while preserving other saved settings, ready for GUI, TUI, and CLI controls. This prevents an older open instance from restoring consent when it saves a theme or recent search. Missing, unreadable, malformed, or unsupported consent fails closed. Invalid settings retain their existing startup error. Persistence lives in `crates/nixbox-config/src/settings.rs`.

Enabling must persist successfully before a worker starts. Disabling first stops the local worker, then persists the choice. If saving fails, keep the current process disabled and show the failure; do not claim the choice was saved or that other instances have stopped. Recheck consent and its revision immediately before transmission. Never rely solely on the settings snapshot taken at startup.

## Shared module and instrumentation

`crates/nixbox-core/src/telemetry.rs` owns observation types, consent checks, aggregation, transport, cancellation, and delivery limits. `telemetry/schema.rs` defines the client/receiver contract. The config crate holds persisted settings and their safe mutation. There are still seven published crates and the existing publish order.

Keep the caller interface small: construct an inert handle with an explicit interface identity, record a typed observation, and refresh consent after a user change. Construction itself must not start collection. Attach the handle to `Session` for the interactive applications and pass it through CLI dispatch to activation execution. `Engine::load`, `Engine::with_config`, and library use must not start telemetry implicitly. Avoid a process-global singleton.

| Location | Planned change |
| --- | --- |
| Workspace and core Cargo manifests, `Cargo.lock` | Add a direct HTTP dependency with only the needed TLS/JSON capabilities. Review the effect on CLI/TUI package size and Nix builds. |
| `crates/nixbox/src/native.rs` | Own the GUI handle and worker lifetime. Count a start after the window opens successfully. The askpass branch exits before telemetry setup. |
| `crates/nixbox-tui/src/main.rs`, `app.rs` | Own the TUI lifetime and count a start after terminal initialization succeeds. |
| `crates/nixbox-cmd/src/lib.rs`, `cli.rs` | Own the CLI lifetime and pass the handle explicitly through command execution. Help, version, completions, config controls, and telemetry inspection produce no reports. |
| `crates/nixbox-core/src/session.rs` | Record one outcome when an active attempt receives its terminal `BuildEvent`. Preserve start time, target, and activation kind in `ActiveBuild`; ignore duplicate terminal events. |
| `crates/nixbox-cmd/src/apply.rs` | Apply the same outcome mapping in `run_activation`, including resumed work. CLI activation bypasses `Session`, so it needs this separate hook. |
| `crates/nixbox/src/launcher.rs`, `askpass.rs` | Keep these paths free of telemetry. Only the launched native application counts a GUI start. |

Do not inspect `Op`, error strings, display labels, `BuildEvent::Line`, structured Nix logs, or generated files to infer telemetry fields. Map typed success/failure/cancellation states directly. Clear timing state on disable and exclude any attempt spanning a consent change, so opting in midway through a rebuild does not collect earlier activity.

## User controls

GUI Settings has a "Share basic usage data" row and an offline "Copy example report" action. TUI Settings has a Usage data page with Off, On, and a synthetic example report. Both default to Off and show the allowed categories, excluded content, effective status, and recipient. The privacy control stays usable during rebuilds and repository operations; Ctrl-S opens that page directly while a TUI repository operation is busy. Enabling is unavailable until the recipient is configured.

Extend the existing CLI configuration interface:

```bash
nixbox config set telemetry-enabled true
nixbox config set telemetry-enabled false
nixbox config get telemetry-enabled
```

`set` accepts only `true` or `false` and calls the shared consent mutation function. An explicit enable command records consent without another prompt. `get` and `config show` expose the saved choice. Add `nixbox telemetry status` to show the effective choice and disabling reason, with `--json` support. Add `nixbox telemetry preview` to print a synthetic report through the real serializer, even while disabled. Neither command collects data or contacts the receiver. Include the same explanation and example in GUI/TUI settings so those users do not need the separate CLI installed.

## Delivery and receiver

Use a small endpoint controlled by the project. The selected hostname is `nixb.duckdns.org`, resolving to `148.113.1.66`. The provider forwards public TCP port 58443 to port 18443 inside the VM. Keep the production recipient fixed and disclosed; do not add an environment variable that redirects reports to arbitrary production destinations.

The recipient constant is `nixbox_core::telemetry::RECIPIENT`, set to `Some("https://nixb.duckdns.org:58443/v1/telemetry")`. GUI, TUI, CLI, Cargo installs, and Nix packages use the same shared core code. Existing published binaries need an upgrade containing this change; server configuration cannot modify an installed binary's recipient or consent. The hostname is resolved normally at delivery time, so moving the VPS only requires updating its DNS record. Changing the recipient itself requires a source change, a consent version bump, and a new application release. Editing settings cannot bypass production disabling overrides.

The current wire format is NixBox's custom JSON schema, not OpenTelemetry or OTLP. The Axum endpoint accepts that schema only. An OpenTelemetry Collector cannot ingest these reports directly.

Use a direct HTTP dependency with JSON and Rustls, without an analytics SDK or automatic tracing export. Inject a receiver adapter and clock at internal test seams. Explicitly configure HTTPS-only delivery, no redirects, no cookies, a fixed application user agent, a one-second connection timeout, and a two-second overall deadline. Do not spawn `curl` or reuse authenticated GitHub clients. Reqwest supports these controls, and its documented defaults require explicit [redirect](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html#method.redirect) and [timeout](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html#method.timeout) configuration.

Record through a bounded channel with `try_send`, which [returns without waiting for capacity](https://docs.rs/tokio/latest/tokio/sync/mpsc/struct.Sender.html#method.try_send). Use at most 64 queued observations and 32 aggregate counter groups. Saturate each count at 1,000 and bound a report to 8 KiB. Drop excess observations. Persist no pending reports or event history. The local consent revision belongs only to internal queue state and must never enter the payload.

For GUI/TUI, schedule the first report after startup without blocking it, then flush at most once per minute. A shutdown flush must respect the same cadence. For CLI, send one aggregate report at command completion. On exit, permit at most 100 ms of total asynchronous delivery time, including any request already running, then cancel remaining work. Reuse an existing in-flight send instead of submitting duplicate counters. Discard reports after any failed delivery, timeout, rate-limit response, or cancellation. Use no retries or offline replay. Short CLI commands and shutdown may lose reports; that is an accepted limit of this design.

The receiver must reject unknown fields, invalid enums, unsupported schema versions, out-of-range counts, and bodies over 8 KiB. Aggregate accepted counts directly into daily totals by the allowed dimensions, then discard request bodies. Keep aggregate totals for 90 days initially. Retain no raw reports or IP addresses in application, reverse-proxy, CDN, error, or request-body logs. Any source-IP abuse limits must use transient memory with a maximum ten-minute expiry. Do not enrich IPs with geolocation or send them to another analytics provider. Do not embed a purported secret ingestion key in the public client.

Before connecting release builds, verify the real hosting provider's logging and retention behavior. Document any unavoidable infrastructure retention before presenting the enable control. If the intended retention cannot be met, resolve the hosting choice first. Accepted aggregate counts cannot be attributed to a device for individual deletion; explain this along with the retention period. A server outage or rate limit silently drops the report on the client.

## Running the Axum receiver

The receiver is an optional example in `nixbox-core`, with Axum and bundled SQLite behind the `telemetry-receiver` feature. Normal GUI/TUI/CLI builds do not include those server dependencies. It writes only daily aggregate counters, without raw reports, IP addresses, client timestamps, or request logs.

```bash
devenv shell -- cargo run -p nixbox-core --features telemetry-receiver --example telemetry-receiver
```

By default it binds `127.0.0.1:8787` and creates a private `nixbox-telemetry.sqlite3` database in the working directory. Set `NIXBOX_TELEMETRY_BIND` and `NIXBOX_TELEMETRY_DATABASE` in the receiver process to change these. Its parent database directory must already exist. Production builds use `cargo build -p nixbox-core --features telemetry-receiver --example telemetry-receiver --release`; the executable is `target/release/examples/telemetry-receiver`.

`POST /v1/telemetry` accepts the schema shown by `nixbox telemetry preview`; `GET /health` returns 204. Accepted reports return 204, malformed or unsupported reports 400, bodies exceeding 8 KiB 413, and the global rate limit 429. Capacity or storage failures return 503. Errors never echo report content. The receiver permits 600 requests per minute, 32 concurrent HTTP requests, eight queued/running database writes, and at most 4,096 aggregate groups per UTC day. HTTP handling has a five-second deadline. These limits use no client addresses or persistent rate-limit records.

`daily_counts` groups by UTC day, release, interface, OS, architecture, and the typed observation. Repeated reports increment `total`; an individual total saturates at one trillion. The receiver retains the current UTC day plus the preceding 89 days. Cleanup runs on startup, ingestion, and every minute while idle, so expiry may lag the UTC day boundary by up to one minute. SQLite uses secure deletion and a delete journal. No public dashboard or query endpoint exposes the database; maintainers query it locally.

For production, run the receiver behind an HTTPS proxy with access logs and body capture disabled, and apply the same retention to backups and infrastructure logs. The receiver does not provide TLS termination or authenticate public ingestion with a client secret. The deployed Nginx container terminates HTTPS and forwards to the private Axum container.

## VPS deployment with Docker and Nginx

The files in [`deploy/telemetry`](../deploy/telemetry) build the receiver and Nginx as separate images. Docker Compose is stored as JSON so its indentation uses tabs; invoke it with `-f compose.json`. The receiver runs as UID 10001, with a read-only root filesystem, no Linux capabilities, and no published host port. Its `/data` directory uses the persistent `counts` volume. The standalone stack publishes Nginx on ports 80 and 58443. Both containers restart unless explicitly stopped; enable the Docker daemon at boot on the VPS. The current shared VM uses the separate NAT setup below.

For the standalone stack, use a Linux VPS with Docker Engine and the Compose plugin. Point `nixb.duckdns.org` at the VPS's public address, keep that DuckDNS record updated if the address changes, and allow inbound TCP 80 and 58443 in the host and provider firewalls. HTTP-01 certificate validation needs public port 80. The NAT setup does not need public port 80 and can coexist with an application already using ports 80 and 443.

Copy this checkout to `/opt/nixbox` on the VPS, then run:

```bash
cd /opt/nixbox/deploy/telemetry
cp .env.example .env
# Optionally set ACME_EMAIL in .env for the certificate account.
./manage.sh init
```

`init` builds the images, starts the receiver and Nginx, and requests a [Let's Encrypt](https://letsencrypt.org/getting-started/) certificate with [Certbot's webroot challenge](https://eff-certbot.readthedocs.io/en/stable/using.html#webroot). Without a certificate, Nginx serves only HTTP redirects and ACME challenge files; collection is unavailable over HTTP. After issuance, the script restarts Nginx with TLS. Certificate issuance fails if the hostname does not reach this VPS on port 80. Set an optional ACME email in `.env`; otherwise Certbot registers without email. Certificate and account state persist in the `certificates` volume. No DuckDNS account token is required by these containers.

Verify the public service after initialization:

```bash
curl --fail --head https://nixb.duckdns.org:58443/health
nixbox telemetry preview > /tmp/nixbox-example.json
curl --fail --include -H 'Content-Type: application/json' --data-binary @/tmp/nixbox-example.json https://nixb.duckdns.org:58443/v1/telemetry
./manage.sh status
```

Both HTTP requests should return 204. The second request deliberately adds synthetic counts to the receiver. It is a maintainer deployment check and does not enable collection on an installation. Release users enable it separately in Settings or with `nixbox config set telemetry-enabled true` after reviewing the disclosure.

Install the supplied renewal timer after the initial certificate succeeds:

```bash
sudo install -m 0644 nixbox-telemetry-renew.service nixbox-telemetry-renew.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now nixbox-telemetry-renew.timer
sudo systemctl list-timers nixbox-telemetry-renew.timer
```

The timer invokes `manage.sh renew` twice daily with randomized delay and catches up after downtime. Certbot renews when due, then Nginx reloads the certificate. The unit assumes `/opt/nixbox`; edit `WorkingDirectory` and `ExecStart` if the checkout lives elsewhere. Renewal failure leaves the currently loaded certificate in place. Check `journalctl -u nixbox-telemetry-renew.service` for failures. Certificate operations contain the server hostname and ACME account information, but do not receive telemetry request bodies.

Use `./manage.sh up` to rebuild and start after an update. Nginx refreshes the receiver's container DNS address every ten seconds. `./manage.sh down` stops the stack and preserves volumes. Avoid `docker compose down --volumes` when preserving aggregates or certificates. To query the database offline, stop the receiver and inspect `counts.sqlite3` in the Compose `counts` volume. Backups must follow the same 90-day aggregate retention; old snapshots can retain counters after expiry in the live database.

The supplied Nginx config disables access logs and directs its error log to `/dev/null`, avoiding client addresses and request data in proxy logs. It forwards only the headers needed for JSON ingestion, without forwarding client IPs, authentication, cookies, or user agents. Request buffering to disk, proxy retries, and caching are disabled. It applies the receiver's 8 KiB body limit and permits collection only on the exact HTTPS path. Unknown HTTPS paths return 404. Provider firewalls, external proxies, CDN logs, and VPS backups remain operator-controlled and need separate retention verification before distributing a release.

`TELEMETRY_DOMAIN` configures Nginx and certificate issuance, not installed applications. Keep it at `nixb.duckdns.org` for this release. `HTTP_PORT` and `HTTPS_PORT` select host bindings; the standalone defaults are 80 and 58443. `TELEMETRY_PUBLIC_HTTPS_PORT` sets the port in HTTP redirects and defaults to 58443. Neither server environment variables nor Docker configuration can enable telemetry in an installation.

## Current shared VM and NAT deployment

The collection server runs in the `nixbox-telemetry` Compose project at `/opt/nixbox-telemetry`. The VM has private IPv4 `10.10.10.140`; the provider's shared IPv4 is `148.113.1.66`. The provider forwards public TCP 58443 to VM port 18443. Public ports 80 and 443 reach the provider's Caddy service, while SlideSage owns those ports inside the VM. Telemetry uses neither existing proxy. Its Nginx publishes only port 18443, and its receiver has no published port. Each telemetry container has a half-CPU quota; receiver and Nginx memory limits are 128 MiB and 64 MiB.

[`compose.nat.json`](../deploy/telemetry/compose.nat.json) uses prebuilt images so compiling Rust does not load the 1 GiB VM. Build the images with the supplied Dockerfiles on a build machine, save and transfer them, load them with `docker load`, and set their tags in `.env` using `.env.nat.example`. Place the NAT Compose file, `manage-nat.sh`, and `duckdns-acme.sh` in `/opt/nixbox-telemetry`. Set a provider TCP forwarding rule with public port 58443, internal port 18443, and no source-IP restriction.

Certificates use the [acme.sh DuckDNS integration](https://github.com/acmesh-official/acme.sh/blob/master/dnsapi/dns_duckdns.sh) and Let's Encrypt DNS-01 validation, which works without public port 80. Store the DuckDNS token in `/opt/nixbox-telemetry/secrets/duckdns-token`, with parent directory mode `0700` and file mode `0600`. The Compose secret exposes it only to the one-shot ACME container. The `acme_state` volume holds the ACME account, renewal configuration, and saved DNS credentials; its directory and configuration files are restricted to root. No credential belongs in source control or an image build context.

```bash
cd /opt/nixbox-telemetry
./manage-nat.sh init
sudo install -m 0644 nixbox-telemetry-nat-renew.service nixbox-telemetry-nat-renew.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now nixbox-telemetry-nat-renew.timer
./manage-nat.sh status
```

`manage-nat.sh init` obtains and installs the certificate, then starts only the telemetry containers. `renew` checks the existing ACME state and gracefully reloads telemetry's Nginx after a successful check. The timer runs twice daily with randomized delay and catches up after downtime. `up` starts the loaded images without compiling or pulling replacements. `down` stops only this Compose project and preserves its volumes. The original receiver-only bootstrap Compose file on the VM is not the active deployment; operational commands use `compose.nat.json` explicitly.

On 2026-10-22, public HTTPS verification succeeded with normal certificate verification. The certificate names `nixb.duckdns.org`, was issued by Let's Encrypt, and expires on 2027-01-20. Health returned 204. Two synthetic reports with release `0.0.0` returned 204 and produced exactly two SQLite counter groups, each with count two. Malformed input returned 400, an 8,193-byte body returned 413, and an unknown path returned 404. Receiver and Nginx request tests produced no log output. The synthetic rows were removed after verification, leaving no test aggregates in the live store. The renewal service completed successfully and its timer is active.

The same verification compared all six SlideSage container IDs, start times, and restart counts with the baseline; all matched. Checksums of its Compose file, Nginx template, and Cloudflare configuration also matched. Its public site returned HTTP 200 before and after deployment. No existing application container was restarted or reconfigured. Provider-level flow logs and backup retention remain outside application verification; the telemetry application and its Nginx retain no request bodies or client addresses.

## Implementation order and verification

| Step | Work | Completion gate |
| --- | --- | --- |
| 1, implemented | Defaulted consent settings, disabling overrides, safe settings writes, and shared consent mutation. | Tests cover defaults, stale saves after opt-out, invalid settings, failed writes, concurrent writers, and consent revisions. |
| 2, implemented | Typed observations, inert handle, aggregation, and bounded HTTP transport. | Controlled local HTTP tests cover disabled collection, report serialization, opt-out, no redirects/retries, and shutdown cancellation. |
| 3, implemented | GUI/TUI settings and CLI configuration/status/preview controls. | Disclosures and offline examples are available; enabling remains gated on a configured recipient. |
| 4, implemented | Startup and both activation completion paths. | Interactive Session and CLI dispatch pass explicit handles. Attempt tokens exclude starts before consent and consume completion once. |
| 5, implemented locally | Axum ingestion, SQLite aggregates, expiry, and capacity limits. | Local HTTP tests exercise collector-to-receiver-to-SQLite delivery and schema rejection. Public infrastructure logging remains unverified. |
| 6, deployment files implemented | Shared recipient, receiver/Nginx Dockerfiles, Compose storage and networking, certificate bootstrap, and renewal timer. | Public HTTPS, provider retention, and enabled release-package traffic still require deployment and release validation. |

Local validation completed on 2026-10-22. `devenv shell -- just ci` passed formatting, Clippy, and 287 workspace tests, with six skipped. The receiver feature gate passed 14 telemetry tests, including collector-to-Axum-to-SQLite delivery. `devenv shell -- just gui-ci` passed all 32 GUI tests, including rendered privacy controls and copying the example report. The native desktop executable also built and stayed running without startup errors against an isolated configuration.

Initial runtime checks used temporary settings and configuration directories while the recipient was unset. The actual TUI displayed the disclosure and example, and refused enablement without a recipient. The CLI status command made no network calls under `strace`; status, preview, and that refused opt-in created no settings file. The receiver executable returned 204 for health and the CLI-generated example report, 400 for malformed JSON, and 413 for an oversized body. Its SQLite file had mode `0600`. These checks used localhost and do not verify public hosting or enabled release-package behavior.

Deployment validation also completed locally on 2026-10-22. Both Dockerfiles built with Podman in Docker image format. The Docker Compose CLI validated the JSON and ran the stack against Podman's local API with isolated volumes and loopback host ports. HTTP served an ACME challenge file, and Nginx switched to HTTPS after a temporary certificate was installed. Curl trusted that certificate explicitly; no public certificate was requested. Through Nginx, health and valid reports returned 204, malformed reports 400, oversized bodies 413, and unknown paths 404. Requests produced no Nginx log output. After receiver recreation, Nginx resolved the replacement and the same SQLite volume contained two aggregate groups with total count four. The receiver ran as UID 10001 with mode `0600` storage, a read-only root filesystem, dropped capabilities, and no published port. ShellCheck, Compose validation, and systemd unit syntax checks passed. The 287 workspace tests, 14 telemetry tests, and 32 GUI tests passed after configuring the shared recipient and version 2 consent. The CLI displayed that recipient, persisted explicit opt-in and opt-out in temporary settings, and suppressed delivery in the development build. Its status command still made no network calls. Public HTTPS at the selected hostname timed out during the check; deploying the stack, obtaining its public certificate, and checking provider retention remain pending.

Extend existing config, Session, CLI, TUI, and GPUI tests. Add focused telemetry tests for the new module. Cover legacy deserialization, explicit consent versioning, save races, malformed settings, disable during a pending request, rapid disable/enable, full queues, timeout, non-success responses, and shutdown deadlines. Use a serializer/schema assertion with sensitive sentinel strings to guard against accidental inclusion of labels, paths, search text, or errors. Tests must never use the production receiver.

Run `just ci` and `devenv shell -- just gui-ci` when implementation changes exist. Verify traffic at the controlled receiver and at the process network layer for fresh, opted-in, and opted-out runs. Exercise launcher and askpass exclusions, overrides, debug/CI/VM suppression, and a second open instance with stale settings. An offline build alone cannot establish the privacy behavior of the shipped binaries.

User and development docs describe the current controls, recipient, and receiver. Keep public-deployment checks separate from local validation. Record hosted retention evidence before distributing delivery in a release.
