#!/usr/bin/env bash
# The local proof of SDLC.md section 3: fmt, clippy with warnings as errors, the tests, a release build, then
# best-validate against the built door on the fake robot, anonymous and signed in through the real device flow and
# `best-robot approve`; then the rosbridge lane, the same door over a real ROS 2 stack in a container (testbed/rosbridge):
# the adapter's testbed tests, best-validate signed in, and a goal sent through the door and read back as events. The
# last line is GATE GREEN or GATE RED, and nothing else is a pass. CI runs this same script.
#
#   scripts/gate.sh [fmt|clippy|test|build|validate|rosbridge|all]      default: all
#
# Needs cargo, node (npx), curl and docker; on Windows, Git Bash. The doors listen on 127.0.0.1:7881 to 7883 and the
# testbed's rosbridge on 9091, away from a door already serving on 7780, and leave nothing behind: each lane's data
# folder is temporary, and its door and container are stopped.
set -uo pipefail
BEST_VALIDATE_VERSION=0.4.3

cd "$(dirname "$0")/.."
what="${1:-all}"
case "$what" in
  fmt|clippy|test|build|validate|rosbridge|all) ;;
  *) echo "scripts/gate.sh: unknown step '$what'; one of fmt, clippy, test, build, validate, rosbridge, all"; echo "GATE RED"; exit 1 ;;
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
if wants build || wants validate || wants rosbridge; then step "release build" cargo build --release -p best-robot; fi

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
sign_in() {  # base: an agent signs in by the device flow, the person approves with the real `best-robot approve`
  # (BEST_ROBOT_DATA and BEST_ROBOT_LISTEN set to the door's), and the key is printed for the lane to use.
  local base="$1" device
  device=$(curl -sf -X POST "$base/auth/device" -d client_id=gate -d connection=gate-robot -d "agent_label=the gate")
  "$bin" approve "$(echo "$device" | field user_code)" >&2
  curl -sf -X POST "$base/auth/token" -d grant_type=urn:ietf:params:oauth:grant-type:device_code \
    -d "device_code=$(echo "$device" | field device_code)" -d client_id=gate | field access_token
}

lane_anonymous() (
  set -euo pipefail
  data="$(mktemp -d)"; trap 'kill $(jobs -p) 2>/dev/null; rm -rf "$data"' EXIT
  BEST_ROBOT_ROBOT=fake BEST_ROBOT_DATA="$data" BEST_ROBOT_LISTEN=127.0.0.1:7881 "$bin" &
  wait_up $! http://127.0.0.1:7881/.well-known/best
  validate http://127.0.0.1:7881
)

# As an admitted agent, the validator probing every active capability with the key. Nothing prints the key.
lane_signed_in() (
  set -euo pipefail
  data="$(mktemp -d)"; trap 'kill $(jobs -p) 2>/dev/null; rm -rf "$data"' EXIT
  base=http://127.0.0.1:7882
  export BEST_ROBOT_ROBOT=fake BEST_ROBOT_DATA="$data" BEST_ROBOT_LISTEN=127.0.0.1:7882
  "$bin" &
  wait_up $! "$base/.well-known/best"
  key=$(sign_in "$base")
  validate "$base" --api-key "$key" --auth-type apikey --auth-header X-Api-Key --probe-registration
)

# The door over a real ROS 2 stack: the testbed container (rosbridge, a Fibonacci action, a battery topic), the
# adapter's tests against it, then the door serving it, validated signed in, and a goal sent through the door whose
# feedback and result come back as events under its correlation id.
lane_rosbridge() (
  set -euo pipefail
  container=best-robot-gate-rosbridge
  data="$(mktemp -d)"; trap 'kill $(jobs -p) 2>/dev/null; docker rm -f "$container" >/dev/null 2>&1; rm -rf "$data"' EXIT
  docker build -q -t best-robot-rosbridge testbed/rosbridge >/dev/null
  docker rm -f "$container" >/dev/null 2>&1 || true
  docker run -d --name "$container" -p 127.0.0.1:9091:9090 best-robot-rosbridge >/dev/null
  for _ in $(seq 1 120); do
    logs=$(docker logs "$container" 2>&1)
    [[ "$logs" == *"WebSocket server started"* && "$logs" == *"testbed ready"* ]] && break
    sleep 0.5
  done
  [[ "$logs" == *"testbed ready"* ]] || { echo "the testbed did not come up:"; echo "$logs" | tail -20; exit 1; }
  BEST_ROBOT_TEST_ROSBRIDGE=ws://127.0.0.1:9091 BEST_ROBOT_TEST_CONTAINER="$container" \
    cargo test -p rosbridge -- --ignored
  base=http://127.0.0.1:7883
  export BEST_ROBOT_ROBOT=rosbridge BEST_ROBOT_CONFIG=testbed/rosbridge/best-robot.toml
  export BEST_ROBOT_DATA="$data" BEST_ROBOT_LISTEN=127.0.0.1:7883
  "$bin" &
  wait_up $! "$base/.well-known/best"
  key=$(sign_in "$base")
  validate "$base" --api-key "$key" --auth-type apikey --auth-header X-Api-Key
  curl -sf -o /dev/null -X POST "$base/commands" -H "X-Api-Key: $key" -H "Content-Type: application/json" -d '{
    "specversion": "1.0", "id": "gate-fibonacci", "source": "gate", "type": "Fibonacci",
    "datacontenttype": "application/json", "dataschema": "'"$base"'/commands/fibonacci/1.0",
    "time": "2026-11-05T00:00:00Z", "data": { "order": 5 } }'
  for _ in $(seq 1 40); do
    events=$(curl -sf "$base/events?correlationId=gate-fibonacci" -H "X-Api-Key: $key")
    [[ "$events" == *FibonacciSucceeded* ]] && break
    sleep 0.25
  done
  echo "$events" | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{
    const e=JSON.parse(s).events, types=e.map(x=>x.type), last=e[e.length-1];
    console.log("events of the goal:", types.join(" "));
    if(!types.includes("FibonacciProgressed")) throw new Error("no feedback came back as an event");
    if(last.type!=="FibonacciSucceeded" || JSON.stringify(last.data.result.sequence)!=="[0,1,1,2,3,5]") throw new Error("the goal did not end with its result");
  })'
)

if wants validate; then
  step "validate, anonymous" lane_anonymous
  step "validate, signed in through the device flow and best-robot approve" lane_signed_in
fi
if wants rosbridge; then step "rosbridge: the door over a ROS 2 testbed" lane_rosbridge; fi

if [ "$failed" -ne 0 ]; then echo "GATE RED"; exit 1; else echo "GATE GREEN"; fi
