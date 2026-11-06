//! Regression guards for the rust-nostr integration: official NIP-44
//! vectors and wire-format interop between Pokkecon's serde structs and
//! the rust-nostr `nostr` crate. Interop evidence is not a security audit;
//! it proves we invoke the library implementation correctly (right
//! version, salt, padding) and that our wire structs are NIP-01-shaped.
#![allow(dead_code)]

#[cfg(test)]
mod tests {
    use crate::nostr::{nip59 as mine59, xonly_from_hex, Event, NostrKey, Rumor, KIND_CHAT_RUMOR};
    use sha2::{Digest, Sha256};
    use nostr_rs::nips::nip44;
    use nostr_rs::prelude::*;

    fn key_from_u64(n: u64) -> NostrKey {
        let mut b = [0u8; 32];
        b[24..].copy_from_slice(&n.to_be_bytes());
        NostrKey::from_secret_bytes(&b).unwrap()
    }

    #[test]
    fn nip44_official_vector_byte_exact() {
        // From the NIP-44 spec: sec1=1, sec2=2, nonce=1, plaintext "a".
        let a = key_from_u64(1);
        let b = key_from_u64(2);
        let sk = SecretKey::from_slice(&a.secret_bytes()).unwrap();
        let pk = PublicKey::from_hex(&b.public_key_hex()).unwrap();
        let mut nonce = [0u8; 32];
        nonce[31] = 1;
        let payload = nip44::encrypt_with_nonce(&sk, &pk, "a", nip44::Nonce::V2(nonce)).unwrap();
        assert_eq!(
            payload,
            "AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABee0G5VSK0/9YypIObAtDKfYEAjD35uVkHyB0F4DwrcNaCXlCWZKaArsGrY6M9wnuTMxWfp1RTN9Xga8no+kF5Vsb"
        );
        // And our decrypt path (rust-nostr under the hood) opens it.
        let b_xonly = xonly_from_hex(&b.public_key_hex()).unwrap();
        let a_xonly = xonly_from_hex(&a.public_key_hex()).unwrap();
        let pt = crate::nostr::nip44::decrypt(&b, &a_xonly, &payload).unwrap();
        assert_eq!(pt, b"a");
        let _ = b_xonly;
    }

    #[test]
    fn nip44_extended_prefix_vectors() {
        // Boundary vectors from the NIP-44 doc: 'a' repeated, same
        // conversation key + nonce; payload SHA-256 must match.
        let a = key_from_u64(1);
        let b = key_from_u64(2);
        let sk = SecretKey::from_slice(&a.secret_bytes()).unwrap();
        let pk = PublicKey::from_hex(&b.public_key_hex()).unwrap();
        let mut nonce = [0u8; 32];
        nonce[31] = 1;
        let cases = [
            (65535usize, "6d8c2810d1e870fbaa1f0a0937126cca837a15f9260e27060c331d70a3c0bc84"),
            (65536, "b7b4edb36ba92e267d322d56d9aebc22e7fa96ff52e3c12adc07f07a43cbc616"),
            (65537, "eeb7c7c5373894ea2c1547cfd3ccb15d5a0b2d619da852e5c79df792dcc9e435"),
        ];
        for (len, expected) in cases {
            let pt = vec![b'a'; len];
            let payload =
                nip44::encrypt_with_nonce(&sk, &pk, &pt, nip44::Nonce::V2(nonce)).unwrap();
            let digest = Sha256::digest(payload.as_bytes());
            assert_eq!(crate::nostr::hex_encode(&digest), expected, "len {}", len);
        }
    }

    #[test]
    fn event_json_interop() {
        // A rust-nostr event deserializes into our Event and verifies.
        let keys = Keys::generate();
        let ev = EventBuilder::new(Kind::from_u16(20001), "signal").finalize(&keys).unwrap();
        let mine: Event = serde_json::from_str(&ev.as_json()).unwrap();
        assert!(mine.verify().unwrap());
        // And ours verifies in the rust-nostr crate.
        let k = NostrKey::generate().unwrap();
        let my_ev = Event::sign(&k, 1_700_000_000, crate::nostr::KIND_SIGNAL, vec![], "x".into()).unwrap();
        let lib_ev = nostr_rs::prelude::Event::from_json(serde_json::to_string(&my_ev).unwrap()).unwrap();
        lib_ev.verify().unwrap();
    }

    #[test]
    fn rumor_id_serialization_matches_crate() {
        // Our Rumor id preimage must equal the rust-nostr compute_id,
        // or gift-wrapped rumors would fail id checks across impls.
        let k = NostrKey::generate().unwrap();
        let rumor = Rumor::new(&k, 1_700_000_123, KIND_CHAT_RUMOR, vec![vec!["p".into(), "ab".into()]], "hi".into());
        let mut lib_ev = UnsignedEvent::new(
            xonly_from_hex(&k.public_key_hex()).unwrap(),
            Timestamp::from_secs(1_700_000_123),
            Kind::from_u16(KIND_CHAT_RUMOR as u16),
            vec![Tag::parse(vec!["p".to_string(), "ab".to_string()]).unwrap()],
            "hi",
        );
        assert_eq!(rumor.id, lib_ev.id().to_hex());
    }
}
