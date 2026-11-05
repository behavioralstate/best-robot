#!/usr/bin/env bash
# The local proof of SDLC.md section 3: fmt, clippy with warnings as errors, the tests, a release build, then
# best-validate against the built door on the fake robot, anonymous and signed in through the real device flow and
# `best-robot approve`. The last line is GATE GREEN or GATE RED, and nothing else is a pass. CI runs this same script.
#
#   scripts/gate.sh [fmt|clippy|test|build|validate|all]      default: all
#
# Needs cargo, node (npx) and curl; on Windows, Git Bash. The doors listen on 127.0.0.1:7881 and 7882, away from one
# already serving on 7780, and leave nothing behind: each lane's data folder is temporary and its door is stopped.
set -uo pipefail
BEST_VALIDATE_VERSION=0.4.3

cd "$(dirname "$0")/.."
what="${1:-all}"
case "$what" in
  fmt|clippy|test|build|validate|all) ;;
  *) echo "scripts/gate.sh: unknown step '$what'; one of fmt, clippy, test, build, validate, all"; echo "GATE RED"; exit 1 ;;
esac
bin="target/release/best-robot"
failed=0
step() {  # name command...: runs the command, keeps the verdict, never stops the script.
  local name="$1"; shift
  echo "=== $name"
  "$@"
  local rc=$?
  if [ "$rc" -ne 0 ]; then echo "FAILED: $name"; failed=1; fi
}
wants() { [[ "$what" == all || "$what" == "$1" ]]; }

if wants fmt; then step "fmt" cargo fmt --all --check; fi
if wants clippy; then step "clippy" cargo clippy --workspace --all-targets -- -D warnings; fi
if wants test; then step "test" cargo test --workspace; fi
if wants build || wants validate; then step "release build" cargo build --release -p best-robot; fi

export BEST_ROBOT_ROBOT=fake
validate() { npx -y "@behavioralstate/best-validate@$BEST_VALIDATE_VERSION" "$@"; }
wait_up() {  # pid url: the door this lane started answers its manifest within thirty seconds, or the lane fails. The
  # process is checked before the port is trusted, so a door left on the port by an earlier run is never validated.
  local pid="$1" url="$2"
  for _ in $(seq 1 60); do
    kill -0 "$pid" 2>/dev/null || { echo "the door for $url exited before it answered"; return 1; }
    curl -sf "$url" >/dev/null 2>&1 && return 0
    sleep 0.5
  done
  echo "the door at $url did not answer"; return 1
}
field() { node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.parse(s)[process.argv[1]]))' "$1"; }

lane_anonymous() (
  set -euo pipefail
  data="$(mktemp -d)"; trap 'kill $(jobs -p) 2>/dev/null; rm -rf "$data"' EXIT
  BEST_ROBOT_DATA="$data" BEST_ROBOT_LISTEN=127.0.0.1:7881 "$bin" &
  wait_up $! http://127.0.0.1:7881/.well-known/best
  validate http://127.0.0.1:7881
)

# As an admitted agent: the device flow, approval through the real `best-robot approve` with this machine's admin
# token, the credential redeemed, and the validator probing every active capability with it. Nothing prints the key.
lane_signed_in() (
  set -euo pipefail
  data="$(mktemp -d)"; trap 'kill $(jobs -p) 2>/dev/null; rm -rf "$data"' EXIT
  base=http://127.0.0.1:7882
  export BEST_ROBOT_DATA="$data" BEST_ROBOT_LISTEN=127.0.0.1:7882
  "$bin" &
  wait_up $! "$base/.well-known/best"
  device=$(curl -sf -X POST "$base/auth/device" -d client_id=gate -d connection=gate-robot -d "agent_label=the gate")
  "$bin" approve "$(echo "$device" | field user_code)"
  key=$(curl -sf -X POST "$base/auth/token" -d grant_type=urn:ietf:params:oauth:grant-type:device_code \
    -d "device_code=$(echo "$device" | field device_code)" -d client_id=gate | field access_token)
  validate "$base" --api-key "$key" --auth-type apikey --auth-header X-Api-Key --probe-registration
)

if wants validate; then
  step "validate, anonymous" lane_anonymous
  step "validate, signed in through the device flow and best-robot approve" lane_signed_in
fi

if [ "$failed" -ne 0 ]; then echo "GATE RED"; exit 1; else echo "GATE GREEN"; fi
