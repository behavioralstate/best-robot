//! The owner's hold: a person has taken the robot over, and no skill is started until they release it. Kept in the
//! data folder, so a door restarted while the robot is held is still held.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Held {
    /// Why, in the owner's words, or what the door says when they gave none.
    pub reason: String,
    /// When, RFC 3339.
    pub since: String,
}

pub struct Hold {
    file: PathBuf,
    held: Mutex<Option<Held>>,
}

impl Hold {
    /// The hold kept in `dir`, held when a hold was left there.
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        let file = dir.join("hold.json");
        let held = match std::fs::read_to_string(&file) {
            Ok(text) => Some(serde_json::from_str(&text).map_err(|e| {
                std::io::Error::other(format!(
                    "{} is not a hold this door wrote ({e}): remove it to release",
                    file.display()
                ))
            })?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        Ok(Self {
            file,
            held: Mutex::new(held),
        })
    }

    pub fn current(&self) -> Option<Held> {
        self.held.lock().unwrap().clone()
    }

    /// Holds the robot for `reason`, in force here at once, even when it cannot be written down; the second part says
    /// whether it was, so a restart keeps it. A hold already in force keeps its `since`, and its reason when this one
    /// gives none.
    pub fn hold(&self, reason: Option<&str>, since: &str) -> (Held, std::io::Result<()>) {
        let mut current = self.held.lock().unwrap();
        let held = Held {
            reason: reason.map(str::to_string).unwrap_or_else(|| {
                current
                    .as_ref()
                    .map_or_else(|| "its owner gave no reason".into(), |h| h.reason.clone())
            }),
            since: current.as_ref().map_or_else(|| since.into(), |h| h.since.clone()),
        };
        *current = Some(held.clone());
        let written = serde_json::to_string(&held)
            .map_err(std::io::Error::other)
            .and_then(|text| {
                let tmp = self.file.with_extension("json.tmp");
                std::fs::write(&tmp, text)?;
                std::fs::rename(&tmp, &self.file)
            });
        (held, written)
    }

    /// Releases the hold; false when there was none.
    pub fn release(&self) -> std::io::Result<bool> {
        let mut current = self.held.lock().unwrap();
        match std::fs::remove_file(&self.file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(current.take().is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_is_kept_across_a_restart_until_released() {
        let dir = tempfile::tempdir().unwrap();
        let hold = Hold::open(dir.path()).unwrap();
        assert_eq!(hold.current(), None);
        hold.hold(Some("servicing the arm"), "2026-11-05T00:00:00Z").1.unwrap();
        let (held, _) = hold.hold(Some("still servicing it"), "2026-11-05T01:00:00Z");
        assert_eq!(held.since, "2026-11-05T00:00:00Z", "a hold in force keeps its since");
        assert_eq!(
            hold.hold(None, "2026-11-05T02:00:00Z").0.reason,
            "still servicing it",
            "and its reason, given none"
        );
        let again = Hold::open(dir.path()).unwrap();
        assert_eq!(again.current().unwrap().reason, "still servicing it");
        assert!(again.release().unwrap());
        assert_eq!(Hold::open(dir.path()).unwrap().current(), None);
        assert!(!again.release().unwrap(), "nothing left to release");
    }

    #[test]
    fn a_hold_that_cannot_be_written_down_is_in_force_all_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let hold = Hold::open(dir.path()).unwrap();
        // A folder where the file would go: the write fails, as on a read-only or full disk.
        std::fs::create_dir(dir.path().join("hold.json.tmp")).unwrap();
        let (held, written) = hold.hold(Some("servicing the arm"), "2026-11-05T00:00:00Z");
        assert!(written.is_err());
        assert_eq!(hold.current(), Some(held));
    }
}
