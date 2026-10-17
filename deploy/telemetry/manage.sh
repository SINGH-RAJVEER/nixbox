#!/bin/sh
set -eu
cd -- "$(dirname -- "$0")"
if [ ! -f .env ]; then
	printf '%s\n' 'Copy .env.example to .env and check the hostname first.' >&2
	exit 1
fi
set -a
# shellcheck source=/dev/null
. ./.env
set +a
: "${TELEMETRY_DOMAIN:?Set TELEMETRY_DOMAIN in .env}"

compose() {
	docker compose --env-file .env -f compose.json "$@"
}

case "${1:-up}" in
	init)
		compose up -d --build --wait --wait-timeout 90 receiver nginx
		if [ -n "${ACME_EMAIL:-}" ]; then
			set -- --email "$ACME_EMAIL"
		else
			set -- --register-unsafely-without-email
		fi
		compose --profile tools run --rm certbot certonly --webroot --webroot-path /var/www/acme --cert-name telemetry --domain "$TELEMETRY_DOMAIN" --non-interactive --agree-tos --keep-until-expiring "$@"
		compose restart nginx
		compose up -d --wait --wait-timeout 90 receiver nginx
		;;
	up)
		compose up -d --build --wait --wait-timeout 90 receiver nginx
		;;
	renew)
		compose --profile tools run --rm certbot renew --cert-name telemetry --non-interactive
		compose exec -T nginx nginx -s reload
		;;
	status)
		compose ps
		;;
	down)
		compose down
		;;
	*)
		printf '%s\n' 'Usage: manage.sh [init|up|renew|status|down]' >&2
		exit 1
		;;
esac
