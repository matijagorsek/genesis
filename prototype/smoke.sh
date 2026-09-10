#!/usr/bin/env bash
# Smoke test the local endpoint: lists models, then one request per role with timing.
set -euo pipefail
ep=${1:-http://127.0.0.1:8080}

echo "== models"
curl -fsS "$ep/v1/models" | python3 -c 'import sys,json;[print(" -",m["id"]) for m in json.load(sys.stdin)["data"]]'

chat() {
  local model=$1 prompt=$2 t0 t1
  t0=$(date +%s.%N)
  out=$(curl -fsS "$ep/v1/chat/completions" -H 'content-type: application/json' \
    -d "$(python3 -c 'import json,sys;print(json.dumps({"model":sys.argv[1],"messages":[{"role":"user","content":sys.argv[2]}],"max_tokens":120,"stream":False}))' "$model" "$prompt")")
  t1=$(date +%s.%N)
  python3 - "$out" "$t0" "$t1" <<'PY'
import json,sys
d=json.loads(sys.argv[1]); dt=float(sys.argv[3])-float(sys.argv[2])
u=d.get("usage",{}); tok=u.get("completion_tokens",0)
print(f"   {dt:5.1f}s  {tok} tok  ~{tok/dt if dt else 0:4.1f} tok/s")
print("   " + d["choices"][0]["message"]["content"].strip().replace("\n","\n   ")[:400])
PY
}

echo "== fast"
chat fast "In one sentence, what is a Linux distribution?"
echo "== code (first call loads the model; can take a minute)"
chat code "Write a bash one-liner that lists the 5 largest files under the current directory."
echo "== embed"
curl -fsS "$ep/v1/embeddings" -H 'content-type: application/json' \
  -d '{"model":"embed","input":"genesis local ai"}' | python3 -c 'import sys,json;d=json.load(sys.stdin);print("   dims:",len(d["data"][0]["embedding"]))'
echo "== running"
curl -fsS "$ep/running"; echo
