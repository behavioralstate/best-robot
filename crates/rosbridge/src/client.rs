//! One websocket to rosbridge, kept open: reconnected a second after it drops, its topics subscribed again on each
//! connection. Answers are routed by the `id` the request carried; a topic's latest message is kept. While the
//! socket is down, a request is refused at once, and every request waiting for an answer is told the socket is gone.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use futures_util::{SinkExt, StreamExt};
use robot::{Snapshot, Unreachable};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

/// How long one connection attempt may take.
const CONNECT_FOR: Duration = Duration::from_secs(5);
/// How often the socket is pinged.
const PING_EVERY: Duration = Duration::from_secs(2);
/// How long rosbridge may send nothing, not even a pong, before the socket is given up.
const SILENT_FOR: Duration = Duration::from_secs(6);

pub struct Client {
    url: String,
    topics: Vec<(String, String)>,
    out: Mutex<Option<mpsc::UnboundedSender<Message>>>,
    waiting: Mutex<HashMap<String, mpsc::UnboundedSender<Value>>>,
    latest: Mutex<HashMap<String, Snapshot>>,
    connected: watch::Sender<bool>,
    next: AtomicU64,
}

impl Client {
    /// Starts keeping a socket to `url`, subscribed to `topics` (topic, message type).
    pub fn start(url: &str, topics: Vec<(String, String)>) -> Arc<Self> {
        let client = Arc::new(Self {
            url: url.into(),
            topics,
            out: Mutex::new(None),
            waiting: Mutex::new(HashMap::new()),
            latest: Mutex::new(HashMap::new()),
            connected: watch::Sender::new(false),
            next: AtomicU64::new(0),
        });
        tokio::spawn(client.clone().keep());
        client
    }

    /// Waits until the socket is open, `within` at most.
    pub async fn connected(&self, within: Duration) -> Result<(), Unreachable> {
        let mut rx = self.connected.subscribe();
        tokio::time::timeout(within, rx.wait_for(|up| *up))
            .await
            .map(|_| ())
            .map_err(|_| Unreachable(format!("no rosbridge answered at {} within {within:?}", self.url)))
    }

    /// Sends `message`; its answers, when it carries an `id`, arrive on the receiver until [`Client::close`].
    pub fn open(&self, id: &str, message: Value) -> Result<mpsc::UnboundedReceiver<Value>, Unreachable> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.waiting.lock().unwrap().insert(id.into(), tx);
        if let Err(e) = self.send(message) {
            self.close(id);
            return Err(e);
        }
        Ok(rx)
    }

    pub fn close(&self, id: &str) {
        self.waiting.lock().unwrap().remove(id);
    }

    pub fn send(&self, message: Value) -> Result<(), Unreachable> {
        self.send_raw(Message::text(message.to_string()))
    }

    fn send_raw(&self, message: Message) -> Result<(), Unreachable> {
        let out = self.out.lock().unwrap();
        out.as_ref()
            .and_then(|o| o.send(message).ok())
            .ok_or_else(|| Unreachable("the door has no connection to it right now".into()))
    }

    /// Calls a ROS service and returns its values, or why it failed.
    pub async fn call(&self, service: &str, args: Value) -> Result<Result<Value, String>, Unreachable> {
        let id = format!("best-robot-call-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let mut rx = self.open(
            &id,
            json!({ "op": "call_service", "id": id, "service": service, "args": args }),
        )?;
        let answer = rx.recv().await;
        self.close(&id);
        let answer = answer.ok_or_else(|| Unreachable(format!("lost rosbridge at {} during {service}", self.url)))?;
        Ok(match answer["result"].as_bool() {
            Some(true) => Ok(answer["values"].clone()),
            _ => Err(answer["values"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| answer["values"].to_string())),
        })
    }

    pub fn latest(&self, topic: &str) -> Option<Snapshot> {
        self.latest.lock().unwrap().get(topic).cloned()
    }

    async fn keep(self: Arc<Self>) {
        loop {
            let attempt = tokio::time::timeout(CONNECT_FOR, tokio_tungstenite::connect_async(self.url.as_str())).await;
            match attempt.unwrap_or_else(|_| Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed)) {
                Ok((socket, _)) => {
                    tracing::info!(url = %self.url, "connected to rosbridge");
                    self.serve(socket).await;
                    tracing::warn!(url = %self.url, "lost rosbridge; reconnecting");
                }
                Err(e) => tracing::debug!(url = %self.url, error = %e, "rosbridge did not answer"),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn serve(
        &self,
        socket: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ) {
        let (mut sink, mut stream) = socket.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        for (topic, ty) in &self.topics {
            let subscribe = json!({ "op": "subscribe", "id": format!("best-robot-{topic}"), "topic": topic, "type": ty, "queue_length": 1 });
            let _ = tx.send(Message::text(subscribe.to_string()));
        }
        *self.out.lock().unwrap() = Some(tx);
        self.connected.send_replace(true);
        let mut writer = tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                if sink.send(m).await.is_err() {
                    break;
                }
            }
        });
        // A socket whose far end vanished (a powered-off robot, a pulled cable) never closes by itself: it is pinged,
        // and given up once nothing at all has come back for a while, so runs waiting on it are told.
        let mut ping = tokio::time::interval(PING_EVERY);
        let mut heard = Instant::now();
        loop {
            tokio::select! {
                _ = ping.tick() => {
                    if heard.elapsed() > SILENT_FOR {
                        tracing::warn!(url = %self.url, "rosbridge went silent");
                        break;
                    }
                    if self.send_raw(Message::Ping(Default::default())).is_err() {
                        break;
                    }
                }
                _ = &mut writer => break,
                next = stream.next() => match next {
                    Some(Ok(message)) => {
                        heard = Instant::now();
                        if let Message::Text(text) = message {
                            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                                self.route(v);
                            }
                        }
                    }
                    _ => break,
                },
            }
        }
        self.connected.send_replace(false);
        *self.out.lock().unwrap() = None;
        writer.abort();
        // Dropping the senders tells every waiting request the socket is gone.
        self.waiting.lock().unwrap().clear();
    }

    fn route(&self, v: Value) {
        if v["op"] == "publish" {
            if let (Some(topic), Some(msg)) = (v["topic"].as_str(), v.get("msg")) {
                let snapshot = Snapshot {
                    message: msg.clone(),
                    received_at: SystemTime::now(),
                };
                self.latest.lock().unwrap().insert(topic.into(), snapshot);
            }
        } else if let Some(id) = v["id"].as_str() {
            if let Some(tx) = self.waiting.lock().unwrap().get(id) {
                let _ = tx.send(v.clone());
            }
        }
    }
}
