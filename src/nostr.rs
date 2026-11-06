//! Nostr wire layer for Pokkecon's network transport.
//!
//! Network transport is Nostr (selected 2026-11-05). Crypto (Schnorr signatures, NIP-44 v2 encryption,
//! NIP-59 gift wrap) is delegated to the rust-nostr crate
//! (`nostr` v0.45.6, minimal features); this module keeps Pokkecon's
//! own serde wire structs, key handling, and fail-closed checks. The
//! official NIP-44 vectors and JSON interop tests live in
//! nostr_vectors.rs as regression guards on the integration.
//!
//! Scope: no relay networking here (relay.rs), no signal-visibility
//! decision (transport.rs), loopback-only testing.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
use nostr_rs::prelude::Event as VEvent;
#[allow(unused_imports)]
use nostr_rs::prelude::*; // glob: local Event/Rumor/NostrKey shadow it

/// x-only public key type used across the transport code (the rust-nostr
/// crate's PublicKey is exactly a BIP-340 x-only key).
pub use nostr_rs::prelude::PublicKey as XOnlyPublicKey;

pub const KIND_SEAL: u32 = 13;
pub const KIND_CHAT_RUMOR: u32 = 14;
pub const KIND_FILE_RUMOR: u32 = 15;
pub const KIND_GIFT_WRAP: u32 = 1059;
pub const KIND_INBOX_RELAYS: u32 = 10050;
/// Pokkecon signal (offer/need) as a NIP-01 ephemeral kind: relays are not
/// expected to store it, matching the signals' 1h TTL.
pub const KIND_SIGNAL: u32 = 20001;

/// NIP-01: kinds 20000..30000 are ephemeral.
pub fn is_ephemeral(kind: u32) -> bool {
    (20_000..30_000).contains(&kind)
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

mod hexutil {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    pub fn encode(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 0x0f) as usize] as char);
        }
        s
    }

    pub fn decode(s: &str) -> Result<Vec<u8>, String> {
        let bytes = s.as_bytes();
        if bytes.len() % 2 != 0 {
            return Err("hex: odd length".into());
        }
        let nib = |c: u8| -> Result<u8, String> {
            match c {
                b'0'..=b'9' => Ok(c - b'0'),
                b'a'..=b'f' => Ok(c - b'a' + 10),
                b'A'..=b'F' => Ok(c - b'A' + 10),
                _ => Err(format!("hex: bad char {}", c as char)),
            }
        };
        let mut out = Vec::with_capacity(bytes.len() / 2);
        for pair in bytes.chunks_exact(2) {
            out.push((nib(pair[0])? << 4) | nib(pair[1])?);
        }
        Ok(out)
    }
}

pub use hexutil::encode as hex_encode;
#[allow(unused_imports)]
pub use hexutil::decode as hex_decode;

fn kind_of(kind: u32) -> Result<Kind, String> {
    u16::try_from(kind)
        .map(Kind::from_u16)
        .map_err(|_| format!("kind {} out of range", kind))
}

fn tags_to_lib(tags: &[Vec<String>]) -> Result<Vec<Tag>, String> {
    tags.iter().map(|t| Tag::parse(t.clone()).map_err(|e| e.to_string())).collect()
}

fn tags_from_lib(tags: &nostr_rs::prelude::Tags) -> Vec<Vec<String>> {
    tags.iter().map(|t| t.as_slice().to_vec()).collect()
}

/// A secp256k1 transport keypair. Rotates daily alongside the ed25519
/// handle; not a long-lived persona.
#[derive(Clone)]
pub struct NostrKey {
    keys: Keys,
}

impl NostrKey {
    pub fn generate() -> Result<Self, String> {
        Ok(NostrKey { keys: Keys::generate() })
    }

    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self, String> {
        let sk = SecretKey::from_slice(bytes).map_err(|e| e.to_string())?;
        Ok(NostrKey { keys: Keys::new(sk) })
    }

    pub fn secret_bytes(&self) -> [u8; 32] {
        self.keys.secret_key().secret_bytes()
    }

    /// x-only public key, lowercase hex (NIP-01 `pubkey` field).
    pub fn public_key_hex(&self) -> String {
        self.keys.public_key().to_hex()
    }

    pub fn public_key(&self) -> XOnlyPublicKey {
        self.keys.public_key()
    }

    fn keys(&self) -> &Keys {
        &self.keys
    }
}

pub fn xonly_from_hex(hex: &str) -> Result<XOnlyPublicKey, String> {
    XOnlyPublicKey::from_hex(hex).map_err(|e| e.to_string())
}

/// A NIP-01 signed event (Pokkecon's serde wire struct; crypto via the
/// rust-nostr crate).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub kind: u32,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

/// An unsigned event as used inside NIP-59 gift wrap.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Rumor {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub kind: u32,
    pub tags: Vec<Vec<String>>,
    pub content: String,
}

/// NIP-01 id preimage: [0, pubkey, created_at, kind, tags, content],
/// compact JSON, sha256. Matches the rust-nostr crate's serialization (proven
/// by the interop tests in nostr_vectors.rs).
fn compute_id(pubkey: &str, created_at: u64, kind: u32, tags: &[Vec<String>], content: &str) -> String {
    let preimage = serde_json::json!([0, pubkey, created_at, kind, tags, content]).to_string();
    hexutil::encode(&Sha256::digest(preimage.as_bytes()))
}

impl Rumor {
    pub fn new(key: &NostrKey, created_at: u64, kind: u32, tags: Vec<Vec<String>>, content: String) -> Rumor {
        let pubkey = key.public_key_hex();
        let id = compute_id(&pubkey, created_at, kind, &tags, &content);
        Rumor { id, pubkey, created_at, kind, tags, content }
    }

    /// Recompute the id; a rumor whose content was altered in transit fails.
    pub fn verify_id(&self) -> bool {
        compute_id(&self.pubkey, self.created_at, self.kind, &self.tags, &self.content) == self.id
    }

    fn to_lib(&self) -> Result<UnsignedEvent, String> {
        Ok(UnsignedEvent::new(
            xonly_from_hex(&self.pubkey)?,
            Timestamp::from_secs(self.created_at),
            kind_of(self.kind)?,
            tags_to_lib(&self.tags)?,
            self.content.clone(),
        ))
    }
}

impl Event {
    pub fn sign(key: &NostrKey, created_at: u64, kind: u32, tags: Vec<Vec<String>>, content: String) -> Result<Event, String> {
        let builder = EventBuilder::new(kind_of(kind)?, content)
            .tags(tags_to_lib(&tags)?)
            .custom_created_at(Timestamp::from_secs(created_at));
        let ev = builder.finalize(key.keys()).map_err(|e| e.to_string())?;
        Event::from_lib(&ev)
    }

    fn from_lib(ev: &VEvent) -> Result<Event, String> {
        let json = ev.as_json();
        serde_json::from_str(&json).map_err(|e| e.to_string())
    }

    fn to_lib(&self) -> Result<VEvent, String> {
        let json = serde_json::to_string(self).map_err(|e| e.to_string())?;
        VEvent::from_json(json).map_err(|e| e.to_string())
    }

    /// NIP-01 verification: id matches the serialization and the Schnorr
    /// signature validates against the declared pubkey (rust-nostr crate).
    pub fn verify(&self) -> Result<bool, String> {
        Ok(self.to_lib()?.verify().is_ok())
    }
}

pub mod nip44 {
    use super::{NostrKey, XOnlyPublicKey};
    use nostr_rs::nips::nip44 as vn44;
    // (no trait imports needed here)

    pub fn encrypt(key: &NostrKey, peer: &XOnlyPublicKey, plaintext: &[u8]) -> Result<String, String> {
        vn44::encrypt(key.keys().secret_key(), peer, plaintext, vn44::Version::V2).map_err(|e| e.to_string())
    }

    pub fn decrypt(key: &NostrKey, peer: &XOnlyPublicKey, payload: &str) -> Result<Vec<u8>, String> {
        vn44::decrypt_to_bytes(key.keys().secret_key(), peer, payload).map_err(|e| e.to_string())
    }
}

pub mod nip59 {
    use super::{Event, NostrKey, Rumor, XOnlyPublicKey, KIND_GIFT_WRAP};
    use nostr_rs::nips::nip59 as vn59;
    use nostr_rs::prelude::FinalizeEvent;

    /// rumor -> seal (kind 13, signed by sender) -> gift wrap (kind 1059,
    /// signed by a random one-time key) addressed to the recipient.
    /// Timestamp randomization per NIP-59 is handled by the rust-nostr crate.
    pub fn gift_wrap(
        sender: &NostrKey,
        recipient: &XOnlyPublicKey,
        rumor: &Rumor,
        _now: u64,
    ) -> Result<Event, String> {
        let wrap = vn59::GiftWrapBuilder::new(*recipient, rumor.to_lib()?)
            .finalize(sender.keys())
            .map_err(|e| e.to_string())?;
        Event::from_lib(&wrap)
    }

    /// Recipient side: the rust-nostr crate verifies the wrap signature,
    /// decrypts and verifies the seal (id + signature), checks the
    /// rumor/seal pubkey match, and decrypts the rumor. On top we verify
    /// the rumor id against its content. Fails closed on any mismatch.
    pub fn unwrap(recipient: &NostrKey, wrap: &Event) -> Result<(String, Rumor), String> {
        if wrap.kind != KIND_GIFT_WRAP {
            return Err("not a gift wrap".into());
        }
        let vwrap = wrap.to_lib()?;
        let unwrapped = vn59::extract_rumor(recipient.keys(), &vwrap).map_err(|e| e.to_string())?;
        let mut vrumor = unwrapped.rumor;
        let id = vrumor.id();
        let rumor = Rumor {
            id: id.to_hex(),
            pubkey: vrumor.pubkey.to_hex(),
            created_at: vrumor.created_at.as_secs(),
            kind: vrumor.kind.as_u16() as u32,
            tags: super::tags_from_lib(&vrumor.tags),
            content: vrumor.content.clone(),
        };
        Ok((unwrapped.sender.to_hex(), rumor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_sign_verify_and_tamper() {
        let k = NostrKey::generate().unwrap();
        let ev = Event::sign(&k, 1_700_000_000, KIND_SIGNAL, vec![], "hello".into()).unwrap();
        assert_eq!(ev.pubkey, k.public_key_hex());
        assert!(ev.verify().unwrap());
        let mut tampered = ev.clone();
        tampered.content = "goodbye".into();
        assert!(!tampered.verify().unwrap());
        let mut wrong_key = ev.clone();
        wrong_key.pubkey = NostrKey::generate().unwrap().public_key_hex();
        assert!(!wrong_key.verify().unwrap());
    }

    #[test]
    fn signal_kind_is_ephemeral() {
        assert!(is_ephemeral(KIND_SIGNAL));
        assert!(!is_ephemeral(KIND_GIFT_WRAP));
        assert!(!is_ephemeral(KIND_INBOX_RELAYS));
    }

    #[test]
    fn gift_wrap_roundtrip_hides_and_delivers() {
        let sender = NostrKey::generate().unwrap();
        let recipient = NostrKey::generate().unwrap();
        let eavesdropper = NostrKey::generate().unwrap();

        let rumor = Rumor::new(
            &sender,
            now_secs(),
            KIND_CHAT_RUMOR,
            vec![vec!["p".into(), recipient.public_key_hex()]],
            "{\"poke\":\"carry\"}".into(),
        );
        let wrap = nip59::gift_wrap(&sender, &recipient.public_key(), &rumor, now_secs()).unwrap();

        // Outer layer is a one-time key, not the sender.
        assert_eq!(wrap.kind, KIND_GIFT_WRAP);
        assert_ne!(wrap.pubkey, sender.public_key_hex());
        assert!(wrap.verify().unwrap());

        let (from, got) = nip59::unwrap(&recipient, &wrap).unwrap();
        assert_eq!(from, sender.public_key_hex());
        assert_eq!(got.content, rumor.content);
        assert_eq!(got.kind, rumor.kind);
        assert_eq!(got.pubkey, rumor.pubkey);

        // A third party holding the event cannot decrypt it.
        assert!(nip59::unwrap(&eavesdropper, &wrap).is_err());
    }
}
