#!/bin/sh
# recallweave demo — builds a small memory store with the real binary, exercises
# the full lifecycle (assert, dedupe, supersede, TTL, forget, conflicts,
# compaction, verify) and exports a memory-pack.
#
# Run from the project root:  sh examples/demo.sh
# It is deterministic: every command pins --now so ids and output are stable.

set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/release/recallweave"
STORE="$ROOT/examples/demo-store"
PACK="$ROOT/examples/memory-pack.json"

if [ ! -x "$BIN" ]; then
  echo "building release binary..."
  ( cd "$ROOT" && cargo build --release )
fi

rw() { "$BIN" --dir "$STORE" "$@"; }

rm -rf "$STORE"

echo "== weaving semantic + preference + procedural + episodic threads =="
rw add --kind semantic   --content "prod region is us-east-1" --tags infra,region --source user --now 1710000000
# capture the id of that semantic memory for supersession
OLD_ID=$(rw query --kind semantic --json --now 1710000050 | grep -o 'mem_[0-9a-f]*' | head -n1)

echo "== the region changed: supersede the old fact =="
rw supersede --old "$OLD_ID" --content "prod region is eu-west-1" --confidence 0.99 --tags infra,region --now 1710000100

echo "== record some preferences (two of them will conflict) =="
rw add --kind preference --content "prefers concise answers" --tags style --confidence 0.8 --now 1710000200
rw add --kind preference --content "prefers dark theme"      --tags ui    --confidence 0.7 --now 1710000200
rw add --kind preference --content "prefers light theme"     --tags ui    --confidence 0.6 --now 1710000200

echo "== a procedural recipe and an episodic note with a 1-day TTL =="
rw add --kind procedural --content "rotate the API token by running ops runbook step 3" --tags ops --confidence 0.9 --now 1710000300
rw add --kind episodic   --content "deploy failed at 14:03 due to bad env var" --tags infra --confidence 0.6 --ttl 86400 --now 1710000400

echo
echo "== dedupe: re-adding normalized-identical content is a no-op =="
rw add --kind semantic --content "PROD region   is  eu-west-1" --now 1710000450

echo
echo "== list live memories =="
rw list --now 1710000500

echo
echo "== detected conflicts =="
rw conflicts --now 1710000500

echo
echo "== integrity check =="
rw verify

echo
echo "== stats =="
rw stats --now 1710000500

echo
echo "== export a portable memory-pack -> $PACK =="
rw export --out "$PACK" --now 1710000500

echo
echo "done. render it with:  (cd viewer && npm install && npm run build && node dist/cli.js ../examples/memory-pack.json --svg ../docs/assets/tapestry-demo.svg)"
