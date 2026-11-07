#!/usr/bin/env bash
# Gazebo, the TurtleBot3 and Nav2 headless, the camera's image republished as JPEG on /camera/image/compressed, and
# rosbridge on 9090; the container ends when any of them does.
set -e
source /opt/ros/jazzy/setup.bash
ros2 launch nav2_bringup tb3_simulation_launch.py headless:=True use_rviz:=False params_file:=/nav2_params.yaml &
ros2 run image_transport republish --ros-args -p in_transport:=raw -p out_transport:=compressed \
  -r in:=/camera/image -r out/compressed:=/camera/image/compressed &
ros2 launch rosbridge_server rosbridge_websocket_launch.xml port:=9090 &
wait -n
