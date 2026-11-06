//! Pokkecon model <-> Nostr transport adapter.
//!
//! Transport keys: secp256k1 keys derived from the same master secret as
//! the handles, but under their own domain separator, so they rotate on
//! the same daily epoch. Two distinct properties, kept separate:
//! derivation secrecy (without the master secret nobody can derive past
//! or future transport keys or handles) versus protocol-observable
//! linkage (every payload carries a publicly verifiable binding from the
//! claimed handle to the transport key, so anyone holding the payload can
//! link THIS handle to THIS transport key for THIS epoch - that is the
//! design, it is what lets a counterparty trust the sender). Cross-epoch
//! linkage is not prevented by the binding either: nothing here stops an
//! observer who learns the master from re-deriving everything, and the
//! local standing map already links own handles across days by design.
//!
//! Signal visibility is an unresolved product decision (open ephemeral
//! signals vs. matcher-keyed encryption). This adapter implements the
//! open shape only as a wire format - nothing here decides who may
//! subscribe, and no cell tag is added to events. The matcher-keyed
//! shape can reuse the same content payload inside a NIP-59 wrap.
#![allow(dead_code)]

use crate::attest::{self, Identity};
use crate::model::{Category, Cell, Handle, Kind, Signal};
use crate::nostr::{nip59, Event, NostrKey, Rumor, KIND_CHAT_RUMOR, KIND_SIGNAL};
use nostr_rs::prelude::SecretKey;
use crate::nostr::XOnlyPublicKey;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Domain separator for transport key derivation. Versioned; distinct
/// from the ed25519 handle domain in attest.rs.
const TRANSPORT_KEY_DOMAIN: &[u8] = b"pokkecon/epoch-key/nostr/v1";
/// Domain separator for the handle<->transport-key binding signature.
const BINDING_DOMAIN: &[u8] = b"pokkecon/transport-binding/v1";

/// The secp256k1 transport key for an epoch, derived from the master
/// secret. Rotates daily with the handle.
pub fn transport_key(id: &Identity, epoch: u64) -> Result<NostrKey, String> {
    let bytes = id.derive_epoch_secret(TRANSPORT_KEY_DOMAIN, epoch);
    let sk = SecretKey::from_slice(&bytes).map_err(|e| e.to_string())?;
    NostrKey::from_secret_bytes(&sk.secret_bytes())
}

pub fn transport_pubkey_hex(id: &Identity, epoch: u64) -> Result<String, String> {
    Ok(transport_key(id, epoch)?.public_key_hex())
}

/// The message the handle key signs to bind itself to a transport key.
fn binding_message(epoch: u64, transport_pubkey_hex: &str) -> Vec<u8> {
    let mut m = BINDING_DOMAIN.to_vec();
    m.extend_from_slice(&epoch.to_be_bytes());
    m.extend_from_slice(transport_pubkey_hex.as_bytes());
    m
}

pub fn sign_binding(id: &Identity, epoch: u64, transport_pubkey_hex: &str) -> Vec<u8> {
    id.sign(epoch, &binding_message(epoch, transport_pubkey_hex))
}

pub fn verify_binding(who: &Handle, epoch: u64, transport_pubkey_hex: &str, sig: &[u8]) -> bool {
    attest::verify(who, &binding_message(epoch, transport_pubkey_hex), sig)
}

/// Signal payload carried in the event content.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SignalPayload {
    pub id: u64,
    pub who: Handle,
    pub kind: Kind,
    pub category: Category,
    pub cell: Cell,
    pub expires_secs: u64,
    pub epoch: u64,
    pub binding_sig: String,
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Package a live signal as a NIP-01 ephemeral event (kind 20001). The
/// content carries the handle and its binding signature; the event's own
/// pubkey is the transport key, which is how counterparties learn it.
/// A NIP-40 expiration tag mirrors the signal TTL (relays SHOULD drop
/// expired events; informational only - TTL enforcement stays local).
pub fn signal_to_event(id: &Identity, epoch: u64, signal: &Signal) -> Result<Event, String> {
    let tk = transport_key(id, epoch)?;
    let tpk = tk.public_key_hex();
    let payload = SignalPayload {
        id: signal.id,
        who: signal.who.clone(),
        kind: signal.kind,
        category: signal.category.clone(),
        cell: signal.cell.clone(),
        expires_secs: secs(signal.expires),
        epoch,
        binding_sig: crate::nostr::hex_encode(&sign_binding(id, epoch, &tpk)),
    };
    let content = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    Event::sign(
        &tk,
        payload.expires_secs.saturating_sub(3600),
        KIND_SIGNAL,
        vec![vec!["expiration".into(), payload.expires_secs.to_string()]],
        content,
    )
}

/// Verify and unpack a signal event. Fails closed on any mismatch:
/// bad event signature, bad binding, wrong kind, or expired signal.
pub fn event_to_signal(ev: &Event, now: SystemTime) -> Result<Signal, String> {
    if ev.kind != KIND_SIGNAL {
        return Err("not a signal event".into());
    }
    if !ev.verify()? {
        return Err("invalid event signature".into());
    }
    let payload: SignalPayload = serde_json::from_str(&ev.content).map_err(|e| e.to_string())?;
    let sig = crate::nostr::hex_decode(&payload.binding_sig)?;
    if !verify_binding(&payload.who, payload.epoch, &ev.pubkey, &sig) {
        return Err("invalid handle binding".into());
    }
    let expires = UNIX_EPOCH + Duration::from_secs(payload.expires_secs);
    let signal = Signal {
        id: payload.id,
        who: payload.who,
        kind: payload.kind,
        category: payload.category,
        cell: payload.cell,
        expires,
    };
    if !signal.live(now) {
        return Err("signal expired".into());
    }
    Ok(signal)
}

/// A private poke wrapped for a recipient's transport key.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PokePayload {
    pub from: Handle,
    pub to: Handle,
    pub text: String,
    pub epoch: u64,
    pub binding_sig: String,
}

pub fn poke_to_wrap(
    id: &Identity,
    epoch: u64,
    to: &Handle,
    recipient_transport: &XOnlyPublicKey,
    text: &str,
    now: u64,
) -> Result<Event, String> {
    let tk = transport_key(id, epoch)?;
    let tpk = tk.public_key_hex();
    let payload = PokePayload {
        from: id.handle(epoch),
        to: to.clone(),
        text: text.to_string(),
        epoch,
        binding_sig: crate::nostr::hex_encode(&sign_binding(id, epoch, &tpk)),
    };
    let rumor = Rumor::new(
        &tk,
        now,
        KIND_CHAT_RUMOR,
        vec![vec!["p".into(), recipient_transport_hex(recipient_transport)]],
        serde_json::to_string(&payload).map_err(|e| e.to_string())?,
    );
    nip59::gift_wrap(&tk, recipient_transport, &rumor, now)
}

fn recipient_transport_hex(x: &XOnlyPublicKey) -> String {
    x.to_hex()
}

/// Unwrap and verify a poke: recovers sender handle and text. Fails
/// closed on bad MAC, bad signatures, wrong recipient, or bad binding.
pub fn unwrap_poke(id: &Identity, epoch: u64, wrap: &Event) -> Result<(Handle, String), String> {
    let tk = transport_key(id, epoch)?;
    let (sender_transport, rumor) = nip59::unwrap(&tk, wrap)?;
    let payload: PokePayload = serde_json::from_str(&rumor.content).map_err(|e| e.to_string())?;
    if payload.to != id.handle(epoch) {
        return Err("poke not addressed to this handle".into());
    }
    let sig = crate::nostr::hex_decode(&payload.binding_sig)?;
    if !verify_binding(&payload.from, payload.epoch, &sender_transport, &sig) {
        return Err("invalid sender binding".into());
    }
    Ok((payload.from, payload.text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nostr::xonly_from_hex;
    use crate::attest::epoch_of;
    use crate::model::SIGNAL_TTL;
    use crate::relay::{Filter, LocalRelay, RelayClient, RelayMessage};
    use std::time::Duration as D;

    fn id(n: u8) -> Identity {
        Identity::from_secret([n; 32])
    }

    fn live_signal(id: &Identity, epoch: u64, n: u64) -> Signal {
        Signal {
            id: n,
            who: id.handle(epoch),
            kind: Kind::Offer,
            category: Category::Carry,
            cell: Cell("u4pruy".into()),
            expires: SystemTime::now() + SIGNAL_TTL,
        }
    }

    #[test]
    fn transport_keys_rotate_with_epoch_and_are_deterministic() {
        let a = id(1);
        let e = epoch_of(SystemTime::now());
        let k1 = transport_pubkey_hex(&a, e).unwrap();
        let k1_again = transport_pubkey_hex(&a, e).unwrap();
        let k2 = transport_pubkey_hex(&a, e + 1).unwrap();
        let other = transport_pubkey_hex(&id(2), e).unwrap();
        assert_eq!(k1, k1_again);
        assert_ne!(k1, k2);
        assert_ne!(k1, other);
        // Distinct from the handle value, different curve.
        assert_ne!(k1, a.handle(e).0);
    }

    #[test]
    fn signal_event_roundtrip_verifies_binding() {
        let alice = id(1);
        let epoch = epoch_of(SystemTime::now());
        let signal = live_signal(&alice, epoch, 7);
        let ev = signal_to_event(&alice, epoch, &signal).unwrap();
        assert!(crate::nostr::is_ephemeral(ev.kind));
        let back = event_to_signal(&ev, SystemTime::now()).unwrap();
        assert_eq!(back.who, signal.who);
        assert_eq!(back.cell, signal.cell);
        // Tampered binding fails closed.
        let mut bad = ev.clone();
        bad.content = bad.content.replace("\"Carry\"", "\"Guide\"");
        assert!(event_to_signal(&bad, SystemTime::now()).is_err());
    }

    #[test]
    fn poke_roundtrip_and_wrong_recipient() {
        let alice = id(1);
        let bob = id(2);
        let carol = id(3);
        let epoch = epoch_of(SystemTime::now());
        let bob_tk = transport_key(&bob, epoch).unwrap();
        let bob_xonly = xonly_from_hex(&bob_tk.public_key_hex()).unwrap();
        let wrap = poke_to_wrap(&alice, epoch, &bob.handle(epoch), &bob_xonly, "carry for you?", 1_700_000_000).unwrap();
        let (from, text) = unwrap_poke(&bob, epoch, &wrap).unwrap();
        assert_eq!(from, alice.handle(epoch));
        assert_eq!(text, "carry for you?");
        assert!(unwrap_poke(&carol, epoch, &wrap).is_err());
    }

    #[test]
    fn full_loop_over_local_relay() {
        let (addr, handle, _relay) = LocalRelay::serve().unwrap();
        let alice = id(1);
        let bob = id(2);
        let epoch = epoch_of(SystemTime::now());

        // Bob's side subscribes first: ephemeral events reach live
        // subscribers only, and this relay never stores them.
        let mut bob_client = RelayClient::connect(&addr).unwrap();
        bob_client.subscribe("signals", vec![Filter { kinds: Some(vec![KIND_SIGNAL]), ..Default::default() }]).unwrap();
        for _ in 0..200 {
            if matches!(bob_client.next_message(D::from_millis(25)), Ok(Some(RelayMessage::Eose { .. }))) { break; }
        }

        // Alice publishes an offer signal (open shape, loopback only).
        let mut alice_client = RelayClient::connect(&addr).unwrap();
        let signal = live_signal(&alice, epoch, 42);
        let ev = signal_to_event(&alice, epoch, &signal).unwrap();
        alice_client.publish(&ev).unwrap();
        let mut heard = None;
        for _ in 0..200 {
            if let Ok(Some(RelayMessage::Event { event, .. })) = bob_client.next_message(D::from_millis(25)) {
                if let Ok(s) = event_to_signal(&event, SystemTime::now()) {
                    heard = Some((event.pubkey.clone(), s));
                    break;
                }
            }
        }
        let (alice_transport, heard_signal) = heard.expect("bob should hear alice's signal");
        assert_eq!(heard_signal.who, alice.handle(epoch));

        // Bob pokes Alice, gift-wrapped, over the same relay.
        let alice_xonly = xonly_from_hex(&alice_transport).unwrap();
        let wrap = poke_to_wrap(&bob, epoch, &heard_signal.who, &alice_xonly, "yes, station gate?", 1_700_000_000).unwrap();
        bob_client.publish(&wrap).unwrap();

        // Alice's inbox catches it by her transport key's p-tag and unwraps.
        let mut alice_inbox = RelayClient::connect(&addr).unwrap();
        let mut inbox_filter = Filter { kinds: Some(vec![crate::nostr::KIND_GIFT_WRAP]), ..Default::default() };
        inbox_filter.tag_filters.insert("#p".into(), vec![transport_pubkey_hex(&alice, epoch).unwrap()]);
        alice_inbox.subscribe("inbox", vec![inbox_filter]).unwrap();
        let mut got = None;
        for _ in 0..200 {
            if let Ok(Some(RelayMessage::Event { event, .. })) = alice_inbox.next_message(D::from_millis(25)) {
                if let Ok(poke) = unwrap_poke(&alice, epoch, &event) {
                    got = Some(poke);
                    break;
                }
            }
        }
        let (from, text) = got.expect("alice should receive bob's poke");
        assert_eq!(from, bob.handle(epoch));
        assert_eq!(text, "yes, station gate?");

        handle.shutdown();
    }
}
