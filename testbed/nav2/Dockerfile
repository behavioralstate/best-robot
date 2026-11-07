# The simulated robot of #263: Gazebo (headless) with a TurtleBot3, Nav2 localising and planning in its world, its
# camera's colour image compressed as JPEG (#266), and rosbridge on 9090. The base is pinned by digest; the ROS packages
# are Jazzy's current release from its apt.
FROM ros:jazzy-ros-base@sha256:8eed0a4669bbf43ee331878a4c39bd8ebddea8e3312a58b4fae8edb14526b617

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ros-jazzy-nav2-bringup ros-jazzy-nav2-minimal-tb3-sim ros-jazzy-rosbridge-server \
      ros-jazzy-compressed-image-transport \
 && rm -rf /var/lib/apt/lists/*

# Nav2's own parameters, with AMCL told where the robot starts (the launch's spawn pose), so it localises without a
# person setting the pose by hand.
RUN sed -e '2a\    set_initial_pose: true' -e '2a\    initial_pose: {x: -2.0, y: -0.5, z: 0.0, yaw: 0.0}' \
      /opt/ros/jazzy/share/nav2_bringup/params/nav2_params.yaml > /nav2_params.yaml

# The waffle's camera is a depth camera, which shows a model no colours: made an RGB-D camera publishing on `camera`,
# its colour image bridged to ROS as /camera/image, which run.sh republishes compressed.
RUN d=/opt/ros/jazzy/share/nav2_minimal_tb3_sim \
 && sed -i 's|<sensor name="intel_realsense_r200_depth" type="depth">|<sensor name="intel_realsense_r200_depth" type="rgbd_camera"><topic>camera</topic>|' \
      $d/urdf/gz_waffle.sdf.xacro \
 && grep -q 'type="rgbd_camera"><topic>camera</topic>' $d/urdf/gz_waffle.sdf.xacro \
 && printf '%s\n' '' '- ros_topic_name: "camera/image"' '  gz_topic_name: "camera/image"' \
      '  ros_type_name: "sensor_msgs/msg/Image"' '  gz_type_name: "gz.msgs.Image"' '  direction: GZ_TO_ROS' \
      >> $d/configs/turtlebot3_waffle_bridge.yaml

COPY run.sh /run.sh
RUN chmod +x /run.sh

EXPOSE 9090
CMD ["/run.sh"]
