#!/bin/sh
# One-time setup on the VPS: writes .env with fresh random keys (if it doesn't exist yet) and
# renders livekit.yaml for your domain.
#
#   ./setup.sh calls.example.org ops@example.org
#
# For a local test on this machine: ./setup.sh localhost ops@example.org --local
set -eu
cd "$(dirname "$0")"
domain="${1:?usage: ./setup.sh <calls domain> <email for TLS certificates> [--local]}"
email="${2:?usage: ./setup.sh <calls domain> <email for TLS certificates> [--local]}"
external=true
[ "${3:-}" = "--local" ] && external=false

if [ ! -f .env ]; then
	umask 077
	key="API$(head -c 9 /dev/urandom | od -An -tx1 | tr -d ' \n')"
	secret="$(head -c 48 /dev/urandom | base64 | tr -d '/+=\n' | cut -c1-48)"
	{
		echo "CALLS_DOMAIN=$domain"
		echo "ACME_EMAIL=$email"
		echo "LIVEKIT_API_KEY=$key"
		echo "LIVEKIT_API_SECRET=$secret"
		echo "BROKER_RATE_GLOBAL_PER_MIN=120"
		echo "BROKER_RATE_IP_PER_MIN=20"
	} > .env
	echo "Wrote .env with new random keys (readable only by you)."
else
	echo "Kept the existing .env."
fi

sed -e "s/__CALLS_DOMAIN__/$domain/g" -e "s/__USE_EXTERNAL_IP__/$external/g" livekit.yaml.in > livekit.yaml
echo "Wrote livekit.yaml for $domain."
