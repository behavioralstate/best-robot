//! The door over a robot: best-peer's BEST surface, with this door's catalogue, its own approver and its data
//! folder, served on a listener the caller binds.

use std::net::SocketAddr;
use std::sync::Arc;

use best_peer_door::{Auth, Door, EventLog};
use robot::Robot;

use crate::approver::OwnerApprover;
use crate::settings::Settings;
use crate::surface;

/// The service key in the manifest, and the `source` of every event.
const SERVICE: &str = "io.best.agents";

const DESCRIPTION: &str = "A robot: what it is doing, where it is and its charge, read by query; its skills become commands as they are exposed.";

pub fn open(settings: &Settings, robot: Arc<dyn Robot>) -> std::io::Result<Door> {
    let approver = OwnerApprover::open(&settings.data)?;
    let auth = Auth::open(&settings.data, &settings.endpoint, Box::new(approver))?;
    let log = EventLog::open(&settings.data)?;
    Ok(Door::new(
        &settings.endpoint,
        SERVICE,
        DESCRIPTION,
        surface::catalogue(robot),
        Arc::new(log),
        Arc::new(auth),
    ))
}

/// Serves the door on `listener` until `stop` resolves.
pub async fn serve(
    listener: tokio::net::TcpListener,
    door: Door,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let app = best_peer_door::router(Arc::new(door));
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(stop)
        .await
}
