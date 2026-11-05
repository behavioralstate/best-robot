//! Who admits an agent: whoever controls the machine the door runs on, as best-peer's local approver decides, with
//! the instructions naming this binary.

use std::path::Path;

use async_trait::async_trait;
use best_peer_door::auth::OAuthError;
use best_peer_door::{Approver, LocalApprover};

pub struct OwnerApprover(LocalApprover);

impl OwnerApprover {
    /// Reads the admin token in `dir`, writing one first when there is none.
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        LocalApprover::open(dir).map(Self)
    }
}

#[async_trait]
impl Approver for OwnerApprover {
    fn instructions(&self, user_code: &str) -> String {
        format!(
            "This robot admits agents on the word of whoever controls the machine its door runs on. On that machine, run: best-robot approve {user_code}"
        )
    }

    async fn attest(&self, proof: &str) -> Result<(), OAuthError> {
        self.0.attest(proof).await
    }

    async fn admit(&self, user_code: &str, proof: &str) -> Result<(), OAuthError> {
        self.0.admit(user_code, proof).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn names_this_binary_and_admits_on_the_machine_token_alone() {
        let dir = tempfile::tempdir().unwrap();
        let approver = OwnerApprover::open(dir.path()).unwrap();
        assert!(approver
            .instructions("ABCD-EFGH")
            .ends_with("best-robot approve ABCD-EFGH"));
        let token = std::fs::read_to_string(dir.path().join("admin-token")).unwrap();
        assert!(approver.attest(token.trim()).await.is_ok());
        assert!(approver.attest("not the token").await.is_err());
    }
}
