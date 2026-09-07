#!/usr/bin/env bash
set -euo pipefail
trap 'printf "[modelport-assurance] setup failed at line %s\n" "$LINENO" >&2' ERR

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
cd "$ROOT_DIR"
umask 077
command -v docker >/dev/null
command -v node >/dev/null
if [[ -z "${MODELPORT_TEST_BINARY:-}" ]]; then
  cargo build --locked --bin model-port
  export MODELPORT_TEST_BINARY="$ROOT_DIR/target/debug/model-port"
fi
[[ -x "$MODELPORT_TEST_BINARY" ]] || { echo 'MODELPORT_TEST_BINARY must be executable' >&2; exit 1; }

runtime_dir="$(mktemp -d)"
postgres_container="modelport-assurance-$$-$RANDOM"
cleanup() {
  docker rm -f -v "$postgres_container" >/dev/null 2>&1 || true
  rm -rf -- "$runtime_dir"
}
trap cleanup EXIT
# shellcheck disable=SC2016
node -e '
const fs = require("node:fs")
const password = require("node:crypto").randomBytes(24).toString("hex")
fs.writeFileSync(process.argv[1], `POSTGRES_USER=modelport\nPOSTGRES_DB=modelport_assurance\nPOSTGRES_PASSWORD=${password}\n`, { mode: 0o600 })
' "$runtime_dir/postgres.env"
printf '%s\n' '[modelport-assurance] starting disposable PostgreSQL'
postgres_port="$(node -e 'const s = require("node:net").createServer(); s.listen(0, "127.0.0.1", () => { process.stdout.write(String(s.address().port)); s.close() })')"
docker run --detach --name "$postgres_container" --label io.modelport.test=assurance \
  --env-file "$runtime_dir/postgres.env" -p "127.0.0.1:$postgres_port:5432" postgres:18.4-alpine >/dev/null
for _ in {1..100}; do
  if docker exec "$postgres_container" pg_isready -h 127.0.0.1 -U modelport -d modelport_assurance >/dev/null 2>&1; then break; fi
  sleep 0.2
done
docker exec "$postgres_container" pg_isready -h 127.0.0.1 -U modelport -d modelport_assurance >/dev/null
postgres_password="$(sed -n 's/^POSTGRES_PASSWORD=//p' "$runtime_dir/postgres.env")"
export MODELPORT_RUNTIME_TEST_DATABASE_URL="postgres://modelport:$postgres_password@127.0.0.1:$postgres_port/modelport_assurance"
export MODELPORT_RUNTIME_TEST_POSTGRES_CONTAINER="$postgres_container"
printf '%s\n' '[modelport-assurance] isolated PostgreSQL, signed OIDC and synthetic loopback upstreams only'
node --test --test-concurrency=1 "$ROOT_DIR/tests/runtime/"*.test.mjs
