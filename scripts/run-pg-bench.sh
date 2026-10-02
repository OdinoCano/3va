#!/usr/bin/env bash
# Start the throwaway Postgres the bench's "100 rows × 100 queries in flight"
# section needs: a docker container named `3va-pg` on 127.0.0.1:$PGPORT
# (default 55432). Idempotent — leaves an already-running container alone.
# bench/run.sh skips the Postgres row (with a note) when no server answers.
set -euo pipefail

NAME=3va-pg
PORT="${PGPORT:-55432}"

if docker inspect "$NAME" >/dev/null 2>&1; then
  if [ "$(docker inspect -f '{{.State.Running}}' "$NAME")" = "true" ]; then
    echo "Postgres already running as $NAME on 127.0.0.1:$PORT" >&2
    exit 0
  fi
  docker rm -f "$NAME" >/dev/null 2>&1 || true
fi

docker run -d --name "$NAME" \
  -e POSTGRES_PASSWORD=bench -e POSTGRES_USER=bench \
  -p "127.0.0.1:$PORT:5432" postgres:16-alpine >/dev/null

for _ in $(seq 1 40); do
  if docker exec "$NAME" pg_isready -U bench >/dev/null 2>&1; then
    if ! docker exec "$NAME" psql -U bench -d postgres -tAc \
      "SELECT 1 FROM pg_database WHERE datname='bench'" | grep -q 1; then
      docker exec "$NAME" createdb -U bench bench
    fi
    echo "Postgres ready on 127.0.0.1:$PORT (container $NAME)" >&2
    exit 0
  fi
  sleep 0.5
done

echo "Postgres container did not become ready" >&2
exit 1