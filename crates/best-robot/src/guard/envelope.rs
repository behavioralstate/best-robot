//! The owner's bounds on where the robot may go and how fast: a geofence it stays inside and zones it never enters, in
//! one frame, and a speed ceiling. Read from the owner's file (`BEST_ROBOT_ENVELOPE`), never from an agent. A goal is
//! held to the fence and the zones; while a skill runs, what the robot reports of where it is and how fast it moves
//! is held to them too.

use std::collections::HashSet;

use robot::Refusal;
use serde::Deserialize;
use serde_json::{json, Value};

/// How far inside the fence, or outside a zone, the place a refusal offers instead lies, in metres.
const MARGIN: f64 = 0.1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    /// The frame every place is in, such as `map`.
    pub frame: String,
    /// The polygon the robot stays inside, as [x, y] corners in order; none means no fence.
    pub geofence: Option<Vec<[f64; 2]>>,
    #[serde(default)]
    pub zones: Vec<Zone>,
    /// Where the robot reports it is, read while a skill runs.
    pub position: Option<Reading>,
    /// How fast the robot reports it moves, and the most it may.
    pub speed: Option<Speed>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zone {
    pub name: String,
    pub polygon: Vec<[f64; 2]>,
}

/// A door topic, and JSON Pointers into its message to x and y.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reading {
    pub topic: String,
    pub x: String,
    pub y: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Speed {
    /// Metres a second.
    pub max: f64,
    pub topic: String,
    /// Pointers to the velocity's x and y, such as `/twist/twist/linear/x`.
    pub x: String,
    pub y: String,
}

impl Envelope {
    pub fn parse(text: &str) -> Result<Self, String> {
        let e: Envelope = toml::from_str(text).map_err(|e| e.to_string())?;
        if e.frame.trim().is_empty() {
            return Err("frame names no frame: say the one every place is in, such as map".into());
        }
        if let Some(fence) = &e.geofence {
            polygon("geofence", fence)?;
        }
        let mut names = HashSet::new();
        for z in &e.zones {
            if z.name.trim().is_empty() || !names.insert(z.name.as_str()) {
                return Err(format!("zones: {:?} needs a name of its own", z.name));
            }
            polygon(&format!("zone {:?}", z.name), &z.polygon)?;
        }
        if let Some(s) = &e.speed {
            if !(s.max.is_finite() && s.max > 0.0) {
                return Err(format!("speed.max {} is not a speed: metres a second, above 0", s.max));
            }
        }
        for (what, pointers) in [
            ("position", e.position.as_ref().map(|p| [&p.x, &p.y])),
            ("speed", e.speed.as_ref().map(|s| [&s.x, &s.y])),
        ] {
            for p in pointers.into_iter().flatten() {
                if !p.starts_with('/') {
                    return Err(format!(
                        "{what}: {p:?} is not a JSON Pointer into the topic's message, such as /pose/pose/position/x"
                    ));
                }
            }
        }
        Ok(e)
    }

    /// The topics the envelope reads, for the door to check it has them.
    pub fn topics(&self) -> Vec<&str> {
        let position = self.position.as_ref().map(|p| p.topic.as_str());
        let speed = self.speed.as_ref().map(|s| s.topic.as_str());
        position.into_iter().chain(speed).collect()
    }

    /// Whether a goal's place may be sent: in the envelope's frame, inside the fence, in no zone. A refusal says the
    /// nearest place that may.
    pub fn place(&self, frame: &str, x: f64, y: f64) -> Result<(), Refusal> {
        if frame != self.frame {
            return Err(refusal(
                "WRONG_FRAME",
                format!(
                    "the goal's place is in frame {frame:?}, and this robot's bounds are in {:?}: send the place in {:?}",
                    self.frame, self.frame
                ),
                json!({ "frame": frame, "expected": self.frame }),
            ));
        }
        self.within(x, y)
    }

    /// Whether the point is inside the fence and in no zone.
    pub fn within(&self, x: f64, y: f64) -> Result<(), Refusal> {
        if let Some(fence) = &self.geofence {
            if !inside(fence, x, y) {
                let (nx, ny) = beyond(fence, x, y);
                return Err(refusal(
                    "OUTSIDE_GEOFENCE",
                    format!(
                        "x {x}, y {y} is outside the geofence this robot's owner set: send a place inside it; the nearest is about x {nx:.2}, y {ny:.2}"
                    ),
                    json!({ "x": x, "y": y, "geofence": fence, "nearest": { "x": nx, "y": ny } }),
                ));
            }
        }
        if let Some(z) = self.zones.iter().find(|z| inside(&z.polygon, x, y)) {
            let (nx, ny) = beyond(&z.polygon, x, y);
            return Err(refusal(
                "IN_FORBIDDEN_ZONE",
                format!(
                    "x {x}, y {y} is in {:?}, a zone this robot's owner forbade: send a place outside it; the nearest is about x {nx:.2}, y {ny:.2}",
                    z.name
                ),
                json!({ "x": x, "y": y, "zone": z.name, "polygon": z.polygon, "nearest": { "x": nx, "y": ny } }),
            ));
        }
        Ok(())
    }
}

fn refusal(code: &'static str, message: String, details: Value) -> Refusal {
    Refusal {
        code,
        message,
        details,
        now: false,
    }
}

fn polygon(what: &str, corners: &[[f64; 2]]) -> Result<(), String> {
    if corners.len() < 3 || corners.iter().flatten().any(|v| !v.is_finite()) {
        return Err(format!(
            "{what} needs three or more [x, y] corners, each a finite number"
        ));
    }
    Ok(())
}

/// Whether (x, y) lies inside the polygon, by the even-odd rule.
fn inside(poly: &[[f64; 2]], x: f64, y: f64) -> bool {
    let mut inside = false;
    for (i, a) in poly.iter().enumerate() {
        let b = poly[(i + 1) % poly.len()];
        if (a[1] > y) != (b[1] > y) && x < a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) {
            inside = !inside;
        }
    }
    inside
}

/// The point nearest (x, y) on the polygon's edge, moved [`MARGIN`] past it, along the way from (x, y) to it: inwards
/// from a point outside, outwards from one inside.
fn beyond(poly: &[[f64; 2]], x: f64, y: f64) -> (f64, f64) {
    let (mut best, mut at) = (f64::INFINITY, (x, y));
    for (i, a) in poly.iter().enumerate() {
        let b = poly[(i + 1) % poly.len()];
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = dx * dx + dy * dy;
        let t = if len2 == 0.0 {
            0.0
        } else {
            (((x - a[0]) * dx + (y - a[1]) * dy) / len2).clamp(0.0, 1.0)
        };
        let p = (a[0] + t * dx, a[1] + t * dy);
        let d = (p.0 - x).hypot(p.1 - y);
        if d < best {
            (best, at) = (d, p);
        }
    }
    let (ux, uy) = if best > 0.0 {
        ((at.0 - x) / best, (at.1 - y) / best)
    } else {
        (0.0, 0.0)
    };
    (at.0 + ux * MARGIN, at.1 + uy * MARGIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENVELOPE: &str = r#"
frame = "map"
geofence = [[-2, -2], [2, -2], [2, 2], [-2, 2]]
[[zones]]
name = "centre"
polygon = [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]]
"#;

    #[test]
    fn a_place_inside_the_fence_and_in_no_zone_may_be_sent() {
        let e = Envelope::parse(ENVELOPE).unwrap();
        assert!(e.place("map", 1.0, 1.0).is_ok());
    }

    #[test]
    fn a_place_outside_the_fence_is_refused_with_the_nearest_inside() {
        let e = Envelope::parse(ENVELOPE).unwrap();
        let r = e.place("map", 5.0, 0.0).unwrap_err();
        assert_eq!(r.code, "OUTSIDE_GEOFENCE");
        let (nx, ny) = (
            r.details["nearest"]["x"].as_f64().unwrap(),
            r.details["nearest"]["y"].as_f64().unwrap(),
        );
        assert!(e.place("map", nx, ny).is_ok(), "{nx} {ny}");
        assert!(r.message.contains("send a place inside it"));
    }

    #[test]
    fn a_place_in_a_zone_is_refused_naming_it_with_the_nearest_outside() {
        let e = Envelope::parse(ENVELOPE).unwrap();
        let r = e.place("map", 0.1, 0.4).unwrap_err();
        assert_eq!(r.code, "IN_FORBIDDEN_ZONE");
        assert_eq!(r.details["zone"], "centre");
        let (nx, ny) = (
            r.details["nearest"]["x"].as_f64().unwrap(),
            r.details["nearest"]["y"].as_f64().unwrap(),
        );
        assert!(e.place("map", nx, ny).is_ok(), "{nx} {ny}");
    }

    #[test]
    fn a_place_in_another_frame_is_refused_naming_the_frame() {
        let e = Envelope::parse(ENVELOPE).unwrap();
        assert_eq!(e.place("odom", 0.0, 1.0).unwrap_err().code, "WRONG_FRAME");
    }

    #[test]
    fn each_mistake_in_the_file_is_refused_naming_it() {
        for (text, said) in [
            (ENVELOPE.replace("[0.5, 0.5], [-0.5, 0.5]", ""), "zone"),
            (ENVELOPE.replace("[2, -2], [2, 2], ", ""), "geofence"),
            (
                format!("{ENVELOPE}[speed]\nmax = 0\ntopic = \"odom\"\nx = \"/x\"\ny = \"/y\"\n"),
                "speed.max",
            ),
            (
                format!("{ENVELOPE}[position]\ntopic = \"pose\"\nx = \"x\"\ny = \"/y\"\n"),
                "JSON Pointer",
            ),
            (ENVELOPE.replace("frame", "fram"), "fram"),
        ] {
            let e = Envelope::parse(&text).unwrap_err();
            assert!(e.contains(said), "{said:?} not in {e:?}");
        }
    }
}
