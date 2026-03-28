#!/bin/bash
#
# ENTRYPOINT for the CI docker image used for testing palpo under complement
# Unlike startup.sh, this waits for PostgreSQL readiness before starting Palpo.

printenv

/etc/init.d/postgresql start

# Wait for PostgreSQL to be ready before starting Palpo
until pg_isready -q; do
    echo "Waiting for PostgreSQL..."
    sleep 1
done
echo "PostgreSQL is ready"

uname -a

# Ensure complement directories exist
mkdir -p /complement/appservice

sed -i "s/your.server.name/${SERVER_NAME}/g" /work/palpo.toml
sed -i "s/your.server.name/${SERVER_NAME}/g" /work/caddy.json
caddy start --config /work/caddy.json > /dev/null
echo "Starting Palpo for ${SERVER_NAME}..."
exec /work/palpo
