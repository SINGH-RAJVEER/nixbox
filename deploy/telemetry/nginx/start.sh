#!/bin/sh
set -eu

: "${TELEMETRY_DOMAIN:?Set TELEMETRY_DOMAIN to the reserved public hostname}"
if [ "${#TELEMETRY_DOMAIN}" -gt 253 ] || ! printf '%s\n' "$TELEMETRY_DOMAIN" | grep -Eq '^([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]([a-z0-9-]{0,61}[a-z0-9])?$'; then
	printf '%s\n' 'TELEMETRY_DOMAIN must be a lowercase DNS hostname' >&2
	exit 1
fi
TELEMETRY_RESOLVER=$(awk '$1 == "nameserver" { print $2; exit }' /etc/resolv.conf)
case "$TELEMETRY_RESOLVER" in
	*:* ) TELEMETRY_RESOLVER="[$TELEMETRY_RESOLVER]" ;;
esac
export TELEMETRY_RESOLVER
TELEMETRY_PUBLIC_HTTPS_PORT=${TELEMETRY_PUBLIC_HTTPS_PORT:-58443}
case "$TELEMETRY_PUBLIC_HTTPS_PORT" in
	''|*[!0-9]*)
		printf '%s\n' 'TELEMETRY_PUBLIC_HTTPS_PORT must be a numeric TCP port' >&2
		exit 1
		;;
esac
if [ "$TELEMETRY_PUBLIC_HTTPS_PORT" -lt 1 ] || [ "$TELEMETRY_PUBLIC_HTTPS_PORT" -gt 65535 ]; then
	printf '%s\n' 'TELEMETRY_PUBLIC_HTTPS_PORT must be between 1 and 65535' >&2
	exit 1
fi
export TELEMETRY_PUBLIC_HTTPS_PORT

mkdir -p /tmp/telemetry-nginx
# Pass literal names so envsubst leaves Nginx request variables untouched.
# shellcheck disable=SC2016
envsubst '${TELEMETRY_DOMAIN} ${TELEMETRY_PUBLIC_HTTPS_PORT}' < /etc/nginx/templates/http.conf.template > /tmp/telemetry-nginx/http.conf
if [ -s /etc/letsencrypt/live/telemetry/fullchain.pem ] && [ -s /etc/letsencrypt/live/telemetry/privkey.pem ]; then
	# shellcheck disable=SC2016
	envsubst '${TELEMETRY_DOMAIN} ${TELEMETRY_RESOLVER}' < /etc/nginx/templates/https.conf.template > /tmp/telemetry-nginx/https.conf
else
	rm -f /tmp/telemetry-nginx/https.conf
	printf '%s\n' 'No certificate yet; serving ACME challenges over HTTP only.'
fi

nginx -t
exec "$@"
