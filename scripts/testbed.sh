# Sourced by scripts/gate.sh and scripts/sim.sh: a testbed container built from testbed/<dir>, started, and waited for.
#
#   testbed_up <dir> <container> <host port> <ready text> <seconds>
#
# Builds the image best-robot-<dir>, starts it as <container> with its rosbridge (9090 inside) on 127.0.0.1:<host
# port>, and returns once its log holds <ready text>, or fails with the log's tail after <seconds>. The caller removes
# the container (docker rm -f) when it is done.
testbed_up() {
  local dir="$1" container="$2" port="$3" ready="$4" within="$5" logs=""
  docker build -q -t "best-robot-$dir" "testbed/$dir" >/dev/null || return 1
  docker rm -f "$container" >/dev/null 2>&1 || true
  docker run -d --name "$container" -p "127.0.0.1:$port:9090" "best-robot-$dir" >/dev/null || return 1
  for _ in $(seq 1 $((within * 2))); do
    logs=$(docker logs "$container" 2>&1)
    [[ "$logs" == *"Rosbridge WebSocket server started"* && "$logs" =~ $ready ]] && return 0
    docker ps -q -f "name=^$container\$" | grep -q . || break
    sleep 0.5
  done
  echo "the $dir testbed did not come up:"; echo "$logs" | tail -20; return 1
}
