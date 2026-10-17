#!/bin/sh
set -eu
umask 077
: "${TELEMETRY_DOMAIN:?Set TELEMETRY_DOMAIN}"
DuckDNS_Token=$(cat /run/secrets/duckdns_token)
test -n "$DuckDNS_Token"
export DuckDNS_Token
chmod 0700 /acme.sh
if [ -f /acme.sh/account.conf ]; then
	chmod 0600 /acme.sh/account.conf
fi
mkdir -p /certificates/live/telemetry

case "${1:-issue}" in
	issue)
		acme.sh --issue --dns dns_duckdns --server letsencrypt --domain "$TELEMETRY_DOMAIN" --keylength ec-256 --dnssleep 60
		acme.sh --install-cert --domain "$TELEMETRY_DOMAIN" --ecc --key-file /certificates/live/telemetry/privkey.pem --fullchain-file /certificates/live/telemetry/fullchain.pem
		;;
	renew)
		acme.sh --cron --server letsencrypt
		;;
	*)
		printf '%s\n' 'Usage: duckdns-acme.sh [issue|renew]' >&2
		exit 1
		;;
esac
find /acme.sh -type f -name '*.conf' -exec chmod 0600 {} +
chmod 0600 /certificates/live/telemetry/privkey.pem
