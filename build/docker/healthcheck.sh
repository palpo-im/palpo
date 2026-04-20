#!/bin/sh

# If PALPO_LISTEN_ADDR is not set try to get the address from the process list
if [ -z "${PALPO_LISTEN_ADDR}" ]; then
  PALPO_LISTEN_ADDR=$(ss -tlpn | awk -F ' +|:' '/palpo/ { print $4 }')
fi

# The actual health check.
# Try HTTP first because most container probes hit the plain app listener,
# then fall back to HTTPS for deployments that terminate TLS in-process.
wget --no-verbose --tries=1 --spider "http://${PALPO_LISTEN_ADDR}/healthz" \
  || wget --no-verbose --tries=1 --spider "https://${PALPO_LISTEN_ADDR}/healthz" \
  || exit 1
