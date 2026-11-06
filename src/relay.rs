//! NIP-01 relay protocol plumbing: message types, a synchronous relay
//! client, and a minimal in-memory relay for local testing.
//!
//! Scope discipline (2026-11-05): this is private plumbing only. Tests
//! bind loopback; no public relay is contacted and no signal content is
//! broadcast. Signal visibility (open ephemeral kind vs. matcher-keyed
//! encryption) is an unresolved product decision - the relay code is
//! deliberately neutral about it. TLS (wss) is out of scope until a real
//! relay deployment exists; the tungstenite TLS feature is off.

#![allow(dead_code)]

use crate::nostr::{is_ephemeral, Event};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tungstenite::{Message, WebSocket};

/// NIP-01 subscription filter. Unknown keys are captured in `tag_filters`
/// ("#e", "#p", ...).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Filter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(flatten)]
    pub tag_filters: HashMap<String, Vec<String>>,
}

impl Filter {
    /// NIP-01 matching. id/author entries are lowercase hex prefixes.
    pub fn matches(&self, ev: &Event) -> bool {
        if let Some(ids) = &self.ids {
            if !ids.iter().any(|p| ev.id.starts_with(p.as_str())) {
                return false;
            }
        }
        if let Some(authors) = &self.authors {
            if !authors.iter().any(|p| ev.pubkey.starts_with(p.as_str())) {
                return false;
            }
        }
        if let Some(kinds) = &self.kinds {
            if !kinds.contains(&ev.kind) {
                return false;
            }
        }
        if let Some(since) = self.since {
            if ev.created_at < since {
                return false;
            }
        }
        if let Some(until) = self.until {
            if ev.created_at > until {
                return false;
            }
        }
        for (key, values) in &self.tag_filters {
            let Some(tag_name) = key.strip_prefix('#') else {
                continue;
            };
            let hit = ev.tags.iter().any(|t| {
                t.first().map(String::as_str) == Some(tag_name)
                    && t.get(1).map_or(false, |v| values.contains(v))
            });
            if !hit {
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClientMessage {
    Event(Event),
    Req { sub_id: String, filters: Vec<Filter> },
    Close { sub_id: String },
}

impl ClientMessage {
    pub fn to_json(&self) -> String {
        match self {
            ClientMessage::Event(ev) => {
                serde_json::json!(["EVENT", ev]).to_string()
            }
            ClientMessage::Req { sub_id, filters } => {
                let mut arr = vec![serde_json::json!("REQ"), serde_json::json!(sub_id)];
                for f in filters {
                    arr.push(serde_json::to_value(f).unwrap_or_default());
                }
                serde_json::Value::Array(arr).to_string()
            }
            ClientMessage::Close { sub_id } => {
                serde_json::json!(["CLOSE", sub_id]).to_string()
            }
        }
    }

    pub fn from_json(text: &str) -> Result<ClientMessage, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let arr = v.as_array().ok_or("not a JSON array")?;
        match arr.first().and_then(|t| t.as_str()) {
            Some("EVENT") => {
                let ev: Event = serde_json::from_value(arr.get(1).cloned().ok_or("missing event")?)
                    .map_err(|e| e.to_string())?;
                Ok(ClientMessage::Event(ev))
            }
            Some("REQ") => {
                let sub_id = arr.get(1).and_then(|s| s.as_str()).ok_or("missing sub id")?.to_string();
                let mut filters = Vec::new();
                for f in &arr[2..] {
                    filters.push(serde_json::from_value(f.clone()).map_err(|e| e.to_string())?);
                }
                Ok(ClientMessage::Req { sub_id, filters })
            }
            Some("CLOSE") => {
                let sub_id = arr.get(1).and_then(|s| s.as_str()).ok_or("missing sub id")?.to_string();
                Ok(ClientMessage::Close { sub_id })
            }
            other => Err(format!("unknown client message: {:?}", other)),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RelayMessage {
    Event { sub_id: String, event: Event },
    Ok { id: String, accepted: bool, message: String },
    Eose { sub_id: String },
    Notice { message: String },
}

impl RelayMessage {
    pub fn to_json(&self) -> String {
        match self {
            RelayMessage::Event { sub_id, event } => {
                serde_json::json!(["EVENT", sub_id, event]).to_string()
            }
            RelayMessage::Ok { id, accepted, message } => {
                serde_json::json!(["OK", id, accepted, message]).to_string()
            }
            RelayMessage::Eose { sub_id } => serde_json::json!(["EOSE", sub_id]).to_string(),
            RelayMessage::Notice { message } => serde_json::json!(["NOTICE", message]).to_string(),
        }
    }

    pub fn from_json(text: &str) -> Result<RelayMessage, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let arr = v.as_array().ok_or("not a JSON array")?;
        match arr.first().and_then(|t| t.as_str()) {
            Some("EVENT") => {
                let sub_id = arr.get(1).and_then(|s| s.as_str()).ok_or("missing sub id")?.to_string();
                let event: Event =
                    serde_json::from_value(arr.get(2).cloned().ok_or("missing event")?).map_err(|e| e.to_string())?;
                Ok(RelayMessage::Event { sub_id, event })
            }
            Some("OK") => Ok(RelayMessage::Ok {
                id: arr.get(1).and_then(|s| s.as_str()).ok_or("missing id")?.to_string(),
                accepted: arr.get(2).and_then(|b| b.as_bool()).ok_or("missing accepted")?,
                message: arr.get(3).and_then(|s| s.as_str()).unwrap_or("").to_string(),
            }),
            Some("EOSE") => Ok(RelayMessage::Eose {
                sub_id: arr.get(1).and_then(|s| s.as_str()).ok_or("missing sub id")?.to_string(),
            }),
            Some("NOTICE") => Ok(RelayMessage::Notice {
                message: arr.get(1).and_then(|s| s.as_str()).unwrap_or("").to_string(),
            }),
            other => Err(format!("unknown relay message: {:?}", other)),
        }
    }
}

type Conn = WebSocket<TcpStream>;

/// A synchronous NIP-01 relay client (loopback or private relay only for
/// now; no TLS feature enabled).
pub struct RelayClient {
    ws: Conn,
}

impl RelayClient {
    pub fn connect(addr: &str) -> Result<RelayClient, String> {
        let stream = TcpStream::connect(addr).map_err(|e| e.to_string())?;
        let (ws, _) = tungstenite::client(format!("ws://{}", addr), stream).map_err(|e| e.to_string())?;
        Ok(RelayClient { ws })
    }

    fn send(&mut self, msg: ClientMessage) -> Result<(), String> {
        self.ws
            .send(Message::Text(msg.to_json()))
            .map_err(|e| e.to_string())
    }

    pub fn publish(&mut self, ev: &Event) -> Result<(), String> {
        self.send(ClientMessage::Event(ev.clone()))
    }

    pub fn subscribe(&mut self, sub_id: &str, filters: Vec<Filter>) -> Result<(), String> {
        self.send(ClientMessage::Req { sub_id: sub_id.to_string(), filters })
    }

    pub fn close_subscription(&mut self, sub_id: &str) -> Result<(), String> {
        self.send(ClientMessage::Close { sub_id: sub_id.to_string() })
    }

    /// Read one relay message, waiting up to `timeout`.
    pub fn next_message(&mut self, timeout: Duration) -> Result<Option<RelayMessage>, String> {
        self.ws
            .get_mut()
            .set_read_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        match self.ws.read() {
            Ok(Message::Text(text)) => RelayMessage::from_json(&text).map(Some),
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(e))
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
            {
                Ok(None)
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

#[derive(Clone)]
struct Subscriber {
    sub_id: String,
    filters: Vec<Filter>,
    out: Sender<RelayMessage>,
}

struct RelayState {
    /// Stored events: everything non-ephemeral. Ephemeral events are only
    /// forwarded to live subscribers, never stored.
    stored: Vec<Event>,
    subs: Vec<Subscriber>,
}

/// A minimal in-memory relay for local testing. Verifies every incoming
/// event (id + Schnorr signature) and fails closed on junk.
pub struct LocalRelay {
    state: Arc<Mutex<RelayState>>,
}

impl LocalRelay {
    pub fn stored_events(&self) -> Vec<Event> {
        self.state.lock().expect("relay state").stored.clone()
    }

    /// Bind a loopback port and serve until the returned handle is dropped.
    /// Returns (addr, shutdown handle).
    pub fn serve() -> io::Result<(String, RelayHandle, LocalRelay)> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = format!("127.0.0.1:{}", listener.local_addr()?.port());
        let state = Arc::new(Mutex::new(RelayState { stored: Vec::new(), subs: Vec::new() }));
        let (stop_tx, stop_rx) = channel::<()>();
        let st = Arc::clone(&state);
        let join = thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("set_nonblocking");
            loop {
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        let st = Arc::clone(&st);
                        thread::spawn(move || {
                            let _ = serve_conn(stream, st);
                        });
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok((addr, RelayHandle { stop: Some(stop_tx), join: Some(join) }, LocalRelay { state }))
    }
}

pub struct RelayHandle {
    stop: Option<Sender<()>>,
    join: Option<thread::JoinHandle<()>>,
}

impl RelayHandle {
    pub fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn serve_conn(stream: TcpStream, state: Arc<Mutex<RelayState>>) -> Result<(), String> {
    // Read side owns the handshaken socket; the write side is a cloned
    // TcpStream so forwarded subscription hits are never blocked behind a
    // blocking read. Only one WebSocket writes, so frames cannot interleave.
    let mut read_ws = tungstenite::accept(stream).map_err(|e| e.to_string())?;
    let write_stream = read_ws.get_mut().try_clone().map_err(|e| e.to_string())?;
    let write_ws = Arc::new(Mutex::new(tungstenite::WebSocket::from_raw_socket(
        write_stream,
        tungstenite::protocol::Role::Server,
        None,
    )));
    let (out_tx, out_rx) = channel::<RelayMessage>();

    // Writer: forwards subscription hits into this connection.
    let ws_writer = Arc::clone(&write_ws);
    let writer = thread::spawn(move || {
        while let Ok(msg) = out_rx.recv() {
            let mut w = ws_writer.lock().expect("ws writer");
            if w.send(Message::Text(msg.to_json())).is_err() {
                break;
            }
        }
    });

    loop {
        let text = match read_ws.read() {
            Ok(Message::Text(t)) => t,
            Ok(Message::Close(_)) => break,
            Ok(_) => continue,
            Err(_) => break,
        };
        match ClientMessage::from_json(&text) {
            Ok(ClientMessage::Event(ev)) => {
                let verified = ev.verify().unwrap_or(false);
                let mut st = state.lock().expect("relay state");
                if verified {
                    if !is_ephemeral(ev.kind) {
                        st.stored.push(ev.clone());
                    }
                    let subs = st.subs.clone();
                    drop(st);
                    for sub in subs {
                        if sub.filters.iter().any(|f| f.matches(&ev)) {
                            let _ = sub.out.send(RelayMessage::Event { sub_id: sub.sub_id.clone(), event: ev.clone() });
                        }
                    }
                    let mut w = write_ws.lock().expect("ws ok");
                    let _ = w.send(Message::Text(
                        RelayMessage::Ok { id: ev.id.clone(), accepted: true, message: String::new() }.to_json(),
                    ));
                } else {
                    let mut w = write_ws.lock().expect("ws ok");
                    let _ = w.send(Message::Text(
                        RelayMessage::Ok { id: ev.id.clone(), accepted: false, message: "invalid: bad id or signature".into() }
                            .to_json(),
                    ));
                }
            }
            Ok(ClientMessage::Req { sub_id, filters }) => {
                let (replay, limit) = {
                    let st = state.lock().expect("relay state");
                    let mut hits: Vec<Event> = Vec::new();
                    for ev in &st.stored {
                        if filters.iter().any(|f| f.matches(ev)) {
                            hits.push(ev.clone());
                        }
                    }
                    hits.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                    let limit = filters.iter().filter_map(|f| f.limit).min().unwrap_or(u64::MAX);
                    (hits, limit)
                };
                {
                    let mut w = write_ws.lock().expect("ws replay");
                    for ev in replay.into_iter().take(limit as usize) {
                        let _ = w.send(Message::Text(
                            RelayMessage::Event { sub_id: sub_id.clone(), event: ev }.to_json(),
                        ));
                    }
                    let _ = w.send(Message::Text(RelayMessage::Eose { sub_id: sub_id.clone() }.to_json()));
                }
                state.lock().expect("relay state").subs.push(Subscriber {
                    sub_id,
                    filters,
                    out: out_tx.clone(),
                });
            }
            Ok(ClientMessage::Close { sub_id }) => {
                state.lock().expect("relay state").subs.retain(|s| s.sub_id != sub_id);
            }
            Err(_) => {
                let mut w = write_ws.lock().expect("ws notice");
                let _ = w.send(Message::Text(
                    RelayMessage::Notice { message: "error: unparseable message".into() }.to_json(),
                ));
            }
        }
    }
    drop(writer);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nostr::{nip59, now_secs, xonly_from_hex, NostrKey, Rumor, KIND_CHAT_RUMOR, KIND_GIFT_WRAP, KIND_SIGNAL};

    fn filter_for_kind(kind: u32) -> Filter {
        Filter { kinds: Some(vec![kind]), ..Default::default() }
    }

    #[test]
    fn req_json_matches_nip01_shape() {
        let mut f = filter_for_kind(KIND_GIFT_WRAP);
        f.tag_filters.insert("#p".into(), vec!["abc".into()]);
        let msg = ClientMessage::Req { sub_id: "s1".into(), filters: vec![f] };
        let v: serde_json::Value = serde_json::from_str(&msg.to_json()).unwrap();
        assert_eq!(v[0], "REQ");
        assert_eq!(v[1], "s1");
        assert_eq!(v[2]["kinds"], serde_json::json!([1059]));
        assert_eq!(v[2]["#p"], serde_json::json!(["abc"]));
        // round trip
        let back = ClientMessage::from_json(&msg.to_json()).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn filter_matching_rules() {
        let k = NostrKey::generate().unwrap();
        let ev = Event::sign(
            &k,
            100,
            KIND_SIGNAL,
            vec![vec!["g".into(), "cell123".into()]],
            "x".into(),
        )
        .unwrap();
        assert!(filter_for_kind(KIND_SIGNAL).matches(&ev));
        assert!(!filter_for_kind(20002).matches(&ev));
        assert!(Filter { since: Some(50), until: Some(150), ..Default::default() }.matches(&ev));
        assert!(!Filter { since: Some(101), ..Default::default() }.matches(&ev));
        let mut tagged = Filter::default();
        tagged.tag_filters.insert("#g".into(), vec!["cell123".into()]);
        assert!(tagged.matches(&ev));
        tagged.tag_filters.insert("#g".into(), vec!["other".into()]);
        assert!(!tagged.matches(&ev));
        let idf = Filter { ids: Some(vec![ev.id[..16].to_string()]), ..Default::default() };
        assert!(idf.matches(&ev));
    }

    fn read_until<F>(client: &mut RelayClient, mut want: F) -> RelayMessage
    where
        F: FnMut(&RelayMessage) -> bool,
    {
        for _ in 0..200 {
            if let Ok(Some(msg)) = client.next_message(Duration::from_millis(25)) {
                if want(&msg) {
                    return msg;
                }
            }
        }
        panic!("timed out waiting for relay message");
    }

    #[test]
    fn relay_fanout_and_ephemeral_not_stored() {
        let (addr, handle, relay) = LocalRelay::serve().unwrap();

        // Subscriber first: ephemeral events only reach live subscribers.
        let mut watcher = RelayClient::connect(&addr).unwrap();
        watcher.subscribe("signals", vec![filter_for_kind(KIND_SIGNAL)]).unwrap();
        read_until(&mut watcher, |m| matches!(m, RelayMessage::Eose { .. }));

        let sender = NostrKey::generate().unwrap();
        let mut pubber = RelayClient::connect(&addr).unwrap();
        let sig_ev = Event::sign(&sender, now_secs(), KIND_SIGNAL, vec![], "{\"offer\":\"carry\"}".into()).unwrap();
        pubber.publish(&sig_ev).unwrap();
        read_until(&mut pubber, |m| matches!(m, RelayMessage::Ok { accepted: true, .. }));

        let got = read_until(&mut watcher, |m| {
            matches!(m, RelayMessage::Event { sub_id, .. } if sub_id == "signals")
        });
        assert!(matches!(got, RelayMessage::Event { event, .. } if event.id == sig_ev.id));
        assert!(relay.stored_events().is_empty(), "ephemeral events must not be stored");

        // A late subscriber gets nothing but EOSE.
        let mut late = RelayClient::connect(&addr).unwrap();
        late.subscribe("late", vec![filter_for_kind(KIND_SIGNAL)]).unwrap();
        let first = read_until(&mut late, |m| matches!(m, RelayMessage::Eose { .. } | RelayMessage::Event { .. }));
        assert!(matches!(first, RelayMessage::Eose { .. }));

        handle.shutdown();
    }

    #[test]
    fn relay_stores_and_replays_regular_events_newest_first() {
        let (addr, handle, _relay) = LocalRelay::serve().unwrap();
        let sender = NostrKey::generate().unwrap();
        let mut pubber = RelayClient::connect(&addr).unwrap();
        let older = Event::sign(&sender, 1000, KIND_GIFT_WRAP, vec![], "a".into()).unwrap();
        let newer = Event::sign(&sender, 2000, KIND_GIFT_WRAP, vec![], "b".into()).unwrap();
        pubber.publish(&older).unwrap();
        pubber.publish(&newer).unwrap();
        read_until(&mut pubber, |m| matches!(m, RelayMessage::Ok { id, .. } if id == &newer.id));

        let mut sub = RelayClient::connect(&addr).unwrap();
        sub.subscribe("s", vec![filter_for_kind(KIND_GIFT_WRAP)]).unwrap();
        let first = read_until(&mut sub, |m| matches!(m, RelayMessage::Event { .. }));
        assert!(matches!(first, RelayMessage::Event { event, .. } if event.id == newer.id));
        handle.shutdown();
    }

    #[test]
    fn relay_rejects_bad_signatures() {
        let (addr, handle, relay) = LocalRelay::serve().unwrap();
        let sender = NostrKey::generate().unwrap();
        let mut client = RelayClient::connect(&addr).unwrap();
        let mut ev = Event::sign(&sender, 1, KIND_GIFT_WRAP, vec![], "x".into()).unwrap();
        ev.content = "forged".into();
        client.publish(&ev).unwrap();
        let msg = read_until(&mut client, |m| matches!(m, RelayMessage::Ok { .. }));
        assert!(matches!(msg, RelayMessage::Ok { accepted: false, .. }));
        assert!(relay.stored_events().is_empty());
        handle.shutdown();
    }

    #[test]
    fn gift_wrapped_poke_flows_over_relay_to_recipient_only() {
        let (addr, handle, _relay) = LocalRelay::serve().unwrap();
        let alice = NostrKey::generate().unwrap();
        let bob = NostrKey::generate().unwrap();
        let bob_xonly = xonly_from_hex(&bob.public_key_hex()).unwrap();

        // Bob subscribes for gift wraps addressed to his transport key.
        let mut bob_client = RelayClient::connect(&addr).unwrap();
        let mut bob_filter = filter_for_kind(KIND_GIFT_WRAP);
        bob_filter.tag_filters.insert("#p".into(), vec![bob.public_key_hex()]);
        bob_client.subscribe("inbox", vec![bob_filter]).unwrap();
        read_until(&mut bob_client, |m| matches!(m, RelayMessage::Eose { .. }));

        // Carol subscribes for her own inbox; she must not see Bob's wrap.
        let carol = NostrKey::generate().unwrap();
        let mut carol_client = RelayClient::connect(&addr).unwrap();
        let mut carol_filter = filter_for_kind(KIND_GIFT_WRAP);
        carol_filter.tag_filters.insert("#p".into(), vec![carol.public_key_hex()]);
        carol_client.subscribe("inbox", vec![carol_filter]).unwrap();
        read_until(&mut carol_client, |m| matches!(m, RelayMessage::Eose { .. }));

        // Alice sends Bob a poke, gift-wrapped, over the relay.
        let rumor = Rumor::new(
            &alice,
            now_secs(),
            KIND_CHAT_RUMOR,
            vec![vec!["p".into(), bob.public_key_hex()]],
            "{\"poke\":\"carry\",\"cell\":\"u4pruy\"}".into(),
        );
        let wrap = nip59::gift_wrap(&alice, &bob_xonly, &rumor, now_secs()).unwrap();
        let mut alice_client = RelayClient::connect(&addr).unwrap();
        alice_client.publish(&wrap).unwrap();
        read_until(&mut alice_client, |m| matches!(m, RelayMessage::Ok { accepted: true, .. }));

        // Bob receives and unwraps it: sender and content recover.
        let got = read_until(&mut bob_client, |m| matches!(m, RelayMessage::Event { sub_id, .. } if sub_id == "inbox"));
        let RelayMessage::Event { event, .. } = got else { panic!("expected event") };
        let (from, recovered) = nip59::unwrap(&bob, &event).unwrap();
        assert_eq!(from, alice.public_key_hex());
        assert_eq!(recovered.content, "{\"poke\":\"carry\",\"cell\":\"u4pruy\"}");

        // Carol gets nothing on her inbox subscription.
        let none = carol_client.next_message(Duration::from_millis(150)).unwrap();
        assert!(none.is_none(), "carol must not receive bob's wrap");

        handle.shutdown();
    }
}
