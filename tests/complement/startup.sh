#!/bin/bash
#
# Default ENTRYPOINT for the docker image used for testing palpo under complement.

set -euo pipefail

printenv

/etc/init.d/postgresql start
until pg_isready -q; do
    echo "Waiting for PostgreSQL..."
    sleep 1
done
echo "PostgreSQL is ready"

uname -a

# Complement mounts application service registrations into this directory.
mkdir -p /complement/appservice

sed -i "s/your.server.name/${SERVER_NAME}/g" /work/palpo.toml
sed -i "s/your.server.name/${SERVER_NAME}/g" /work/caddy.json
caddy start --config /work/caddy.json > /dev/null
echo "Starting Palpo for ${SERVER_NAME}..."
exec env RUST_BACKTRACE=false /work/palpo
