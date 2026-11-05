#!/usr/bin/env bash
# The local proof of SDLC.md section 3: fmt, clippy with warnings as errors, the tests, a release build, then
# best-validate against the built door on the fake robot, anonymous and signed in through the real device flow and
# `best-robot approve`; then the rosbridge lane, the same door over a real ROS 2 stack in a container (testbed/rosbridge):
# the adapter's testbed tests, best-validate signed in, and a goal sent through the door and read back as events; then
# the nav2 lane, the door driving a simulated TurtleBot3 under Nav2 (testbed/nav2) to a pose and back, out of the map,
# and cancelled on the way; then, behind the owner's envelope (testbed/nav2/envelope.toml), goals outside the geofence
# and in a forbidden zone refused with the fix, a key approved for one skill refused another, and `best-robot hold
# --now` stopping a navigation, kept across a restart of the door, until `best-robot release`, and a navigation stopped
# when the robot's odometry reports it above a speed ceiling. The last line is GATE
# GREEN or GATE RED, and nothing else is a pass. CI runs this same script.
#
#   scripts/gate.sh [fmt|clippy|test|build|validate|rosbridge|nav2|all]      default: all
#
# Needs cargo, node (npx), curl and docker; on Windows, Git Bash. The doors listen on 127.0.0.1:7881 to 7884 and the
# testbeds' rosbridges on 9091 and 9093, away from a door on 7780 and scripts/sim.sh's robot on 9092, and leave nothing
# behind: each lane's data folder is temporary, and its door and container are stopped.
set -uo pipefail
BEST_VALIDATE_VERSION=0.4.3

cd "$(dirname "$0")/.."
source scripts/testbed.sh
what="${1:-all}"
case "$what" in
  fmt|clippy|test|build|validate|rosbridge|nav2|all) ;;
  *) echo "scripts/gate.sh: unknown step '$what'; one of fmt, clippy, test, build, validate, rosbridge, nav2, all"; echo "GATE RED"; exit 1 ;;
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
if wants build || wants validate || wants rosbridge || wants nav2; then step "release build" cargo build --release -p best-robot; fi

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
sign_in() {  # base [approve flags...]: an agent signs in by the device flow, the person approves with the real
  # `best-robot approve` (BEST_ROBOT_DATA and BEST_ROBOT_LISTEN set to the door's) and the flags, and the key is
  # printed for the lane to use.
  local base="$1" device; shift
  device=$(curl -sf -X POST "$base/auth/device" -d client_id=gate -d connection=gate-robot -d "agent_label=the gate")
  "$bin" approve "$(echo "$device" | field user_code)" "$@" >&2
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

send() {  # base key schema type id data: sends a command through the door as the signed-in agent.
  curl -sf -o /dev/null -X POST "$1/commands" -H "X-Api-Key: $2" -H "Content-Type: application/json" -d '{
    "specversion": "1.0", "id": "'"$5"'", "source": "gate", "type": "'"$4"'", "datacontenttype": "application/json",
    "dataschema": "'"$1"'/commands/'"$3"'/1.0", "time": "2026-11-05T00:00:00Z", "data": '"$6"' }'
}
answer() {  # base key schema type id data: sends a command as send does, and prints the door's status and body.
  local body; body=$(mktemp)
  curl -s -o "$body" -w '%{http_code} ' -X POST "$1/commands" -H "X-Api-Key: $2" -H "Content-Type: application/json" -d '{
    "specversion": "1.0", "id": "'"$5"'", "source": "gate", "type": "'"$4"'", "datacontenttype": "application/json",
    "dataschema": "'"$1"'/commands/'"$3"'/1.0", "time": "2026-11-05T00:00:00Z", "data": '"$6"' }'
  cat "$body"; rm -f "$body"
}
refused() {  # label status code: reads `answer`'s output from stdin, and fails unless the door refused with that status
  # and error code; prints the message, the fix it names.
  node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{
    const at=s.indexOf(" "), status=s.slice(0,at), b=JSON.parse(s.slice(at+1)||"{}"), e=b.error||{};
    console.log(process.argv[1]+":", status, e.code, "-", e.message);
    if(status!==process.argv[2]||e.code!==process.argv[3]) { console.log(s); process.exit(1); }
  })' "$1" "$2" "$3"
}
events_until() {  # base key correlation pattern seconds: the events under the correlation once one matches the pattern.
  local events=""
  for _ in $(seq 1 $(($5 * 4))); do
    events=$(curl -sf "$1/events?correlationId=$3" -H "X-Api-Key: $2")
    [[ "$events" =~ $4 ]] && break
    sleep 0.25
  done
  echo "$events"
}
expect() {  # label js: reads events from stdin, prints their types, and fails unless the js expression over `e` (the
  # events), `types` and `last` holds.
  node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{
    const e=JSON.parse(s).events, types=e.map(x=>x.type), last=e[e.length-1];
    console.log(process.argv[1]+":", types.join(" "));
    if(!eval(process.argv[2])) { console.log(JSON.stringify(last)); process.exit(1); }
  })' "$1" "$2"
}
door_over() {  # port config [envelope]: starts the door over the rosbridge robot <config> names, on 127.0.0.1:<port>,
  # behind the envelope when one is named, as a job of the calling lane (so its trap stops it, and $! is its pid),
  # with the lane's data folder, and waits until it answers.
  export BEST_ROBOT_ROBOT=rosbridge BEST_ROBOT_CONFIG="$2" BEST_ROBOT_DATA="$data" BEST_ROBOT_LISTEN="127.0.0.1:$1"
  BEST_ROBOT_ENVELOPE="${3:-}" "$bin" &
  wait_up $! "http://127.0.0.1:$1/.well-known/best"
}

# The door over a real ROS 2 stack: the testbed container (rosbridge, a Fibonacci action, a battery topic), the
# adapter's tests against it, then the door serving it, validated signed in, and a goal sent through the door whose
# feedback and result come back as events under its correlation id.
lane_rosbridge() (
  set -euo pipefail
  container=best-robot-gate-rosbridge
  data="$(mktemp -d)"; trap 'kill $(jobs -p) 2>/dev/null; docker rm -f "$container" >/dev/null 2>&1; rm -rf "$data"' EXIT
  testbed_up rosbridge "$container" 9091 'testbed ready' 60
  BEST_ROBOT_TEST_ROSBRIDGE=ws://127.0.0.1:9091 BEST_ROBOT_TEST_CONTAINER="$container" \
    cargo test -p rosbridge -- --ignored
  base=http://127.0.0.1:7883
  door_over 7883 testbed/rosbridge/best-robot.toml
  key=$(sign_in "$base")
  validate "$base" --api-key "$key" --auth-type apikey --auth-header X-Api-Key
  send "$base" "$key" fibonacci Fibonacci gate-fibonacci '{ "order": 5 }'
  events_until "$base" "$key" gate-fibonacci FibonacciSucceeded 10 | expect "events of the goal" \
    'types.includes("FibonacciProgressed") && last.type==="FibonacciSucceeded" && JSON.stringify(last.data.result.sequence)==="[0,1,1,2,3,5]"'
)

# The door over the simulated robot (testbed/nav2: Gazebo, a TurtleBot3, Nav2): validated signed in, then driven
# through the door to a pose and back, sent somewhere it cannot reach, and cancelled on the way, each told as events;
# then restarted behind the owner's envelope, where the door refuses what the owner bounds, and the owner's hold
# stops a run and outlives a restart.
lane_nav2() (
  set -euo pipefail
  container=best-robot-gate-nav2
  # On a failure, Nav2's own complaints, since the container goes with the lane.
  data="$(mktemp -d)"; trap 'rc=$?; kill $(jobs -p) 2>/dev/null
    [ "$rc" -ne 0 ] && docker logs "$container" 2>&1 | grep -iE "error|abort|fail|warn" | tail -40
    docker rm -f "$container" >/dev/null 2>&1; rm -rf "$data"' EXIT
  testbed_up nav2 "$container" 9093 'lifecycle_manager_navigation\]: .{0,20}Managed nodes are active' 300
  sed 's/:9092"/:9093"/' testbed/nav2/best-robot.toml > "$data/nav2.toml"
  base=http://127.0.0.1:7884
  door_over 7884 "$data/nav2.toml"; door=$!
  key=$(sign_in "$base")
  validate "$base" --api-key "$key" --auth-type apikey --auth-header X-Api-Key
  goal() { echo '{ "pose": { "header": { "frame_id": "map" }, "pose": { "position": { "x": '"$1"', "y": '"$2"' }, "orientation": { "w": 1 } } } }'; }
  ended='NavigateTo(Succeeded|Aborted|Canceled|Refused)|CommandFailed'
  send "$base" "$key" navigate-to NavigateTo gate-there "$(goal -0.55 -0.55)"
  events_until "$base" "$key" gate-there "$ended" 120 | expect "to a pose" \
    'types.includes("NavigateToProgressed") && last.type==="NavigateToSucceeded"'
  curl -sf "$base/queries/get-pose" -H "X-Api-Key: $key" | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{
    const p=JSON.parse(s).message.pose.pose.position; console.log("get-pose:", p.x.toFixed(2), p.y.toFixed(2));
    if(Math.hypot(p.x+0.55,p.y+0.55)>0.5) process.exit(1); })'
  send "$base" "$key" navigate-to NavigateTo gate-back "$(goal -2.0 -0.5)"
  events_until "$base" "$key" gate-back "$ended" 120 | expect "and back" 'last.type==="NavigateToSucceeded"'
  send "$base" "$key" navigate-to NavigateTo gate-far "$(goal 10 10)"
  events_until "$base" "$key" gate-far "$ended" 60 | expect "out of the map" \
    'last.type==="NavigateToAborted" && last.data.result.error_code===204'
  send "$base" "$key" navigate-to NavigateTo gate-cancelled "$(goal 1.6 1.6)"
  events_until "$base" "$key" gate-cancelled NavigateToProgressed 30 >/dev/null
  send "$base" "$key" cancel-skill CancelSkill gate-cancel '{ "command": "gate-cancelled" }'
  events_until "$base" "$key" gate-cancel 'SkillCancel(Requested|Refused)' 10 | expect "the cancel" 'last.type==="SkillCancelRequested"'
  events_until "$base" "$key" gate-cancelled "$ended" 60 | expect "cancelled on the way" 'last.type==="NavigateToCanceled"'

  restart() {  # [envelope]: the door again, behind the testbed's envelope or the one named.
    kill "$door"; wait "$door" 2>/dev/null || true
    door_over 7884 "$data/nav2.toml" "${1:-testbed/nav2/envelope.toml}"; door=$!
  }
  restart
  answer "$base" "$key" navigate-to NavigateTo gate-fenced "$(goal 3.0 0.0)" | refused "outside the geofence" 400 OUTSIDE_GEOFENCE
  answer "$base" "$key" navigate-to NavigateTo gate-zoned "$(goal 0.1 0.1)" | refused "in a forbidden zone" 400 IN_FORBIDDEN_ZONE
  narrow=$(sign_in "$base" --may navigate-to)
  answer "$base" "$narrow" spin Spin gate-spin '{ "target_yaw": 1.57, "time_allowance": { "sec": 10 } }' \
    | refused "a key for navigate-to alone, on spin" 403 FORBIDDEN

  ended='NavigateTo(Succeeded|Aborted|Canceled|Refused|Stopped)|CommandFailed'
  send "$base" "$narrow" navigate-to NavigateTo gate-held "$(goal 1.6 1.6)"
  events_until "$base" "$narrow" gate-held NavigateToProgressed 30 >/dev/null
  "$bin" hold --now --reason "the gate takes it over"
  events_until "$base" "$narrow" gate-held "$ended" 60 | expect "stopped by the hold" \
    'last.type==="NavigateToStopped" && last.data.reason.includes("the gate takes it over")'
  answer "$base" "$key" navigate-to NavigateTo gate-while-held "$(goal -2.0 -0.5)" | refused "while held" 409 HELD
  restart
  answer "$base" "$key" navigate-to NavigateTo gate-held-on "$(goal -2.0 -0.5)" | refused "held across a restart" 409 HELD
  "$bin" release
  send "$base" "$key" navigate-to NavigateTo gate-released "$(goal -2.0 -0.5)"
  events_until "$base" "$key" gate-released "$ended" 120 | expect "released, and back" 'last.type==="NavigateToSucceeded"'

  # A ceiling below the 0.5 m/s Nav2 drives at: the run is stopped on the robot's own odometry.
  sed 's/^max = .*/max = 0.2/' testbed/nav2/envelope.toml > "$data/slow.toml"
  restart "$data/slow.toml"
  send "$base" "$key" navigate-to NavigateTo gate-too-fast "$(goal 1.6 1.6)"
  events_until "$base" "$key" gate-too-fast "$ended" 60 | expect "stopped above the speed ceiling" \
    'last.type==="NavigateToStopped" && last.data.reason.includes("above the 0.2 m/s")'
)

if wants validate; then
  step "validate, anonymous" lane_anonymous
  step "validate, signed in through the device flow and best-robot approve" lane_signed_in
fi
if wants rosbridge; then step "rosbridge: the door over a ROS 2 testbed" lane_rosbridge; fi
if wants nav2; then step "nav2: the door drives a simulated robot" lane_nav2; fi

if [ "$failed" -ne 0 ]; then echo "GATE RED"; exit 1; else echo "GATE GREEN"; fi
