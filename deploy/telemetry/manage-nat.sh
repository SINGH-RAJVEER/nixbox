#!/bin/sh
set -eu
cd -- "$(dirname -- "$0")"
set -a
# shellcheck source=/dev/null
. ./.env
set +a

compose() {
	docker compose --env-file .env -f compose.nat.json "$@"
}

case "${1:-up}" in
	init)
		compose --profile tools run --rm --no-deps acme issue
		compose up -d --pull never --wait --wait-timeout 90 receiver nginx
		;;
	up)
		compose up -d --pull never --wait --wait-timeout 90 receiver nginx
		;;
	renew)
		compose --profile tools run --rm --no-deps acme renew
		compose exec -T nginx nginx -t
		compose exec -T nginx nginx -s reload
		;;
	status)
		compose ps
		;;
	down)
		compose down
		;;
	*)
		printf '%s\n' 'Usage: manage-nat.sh [init|up|renew|status|down]' >&2
		exit 1
		;;
esac
