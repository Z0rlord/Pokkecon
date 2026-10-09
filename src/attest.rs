use crate::model::{Category, Handle};
use std::time::SystemTime;

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use sha2::{Digest, Sha256};

/// Seconds per pseudonym epoch. A new handle (and key) every day.
pub const EPOCH_SECS: u64 = 60 * 60 * 24;

pub fn epoch_of(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) / EPOCH_SECS
}

/// A person's long-term secret. Never leaves the device. Per-epoch keys are derived from it,
/// so the public handles of different epochs are not linkable without the master secret.
pub struct Identity {
    master: [u8; 32],
}

impl Identity {
    pub fn from_secret(master: [u8; 32]) -> Self {
        Identity { master }
    }

    fn key(&self, epoch: u64) -> SigningKey {
        let mut h = Sha256::new();
        h.update(b"pokkecon/epoch-key/v1");
        h.update(self.master);
        h.update(epoch.to_be_bytes());
        SigningKey::from_bytes(&h.finalize().into())
    }

    /// The pseudonym for an epoch: the hex of that epoch's ed25519 public key.
    pub fn handle(&self, epoch: u64) -> Handle {
        let pk = self.key(epoch).verifying_key().to_bytes();
        Handle(pk.iter().map(|b| format!("{:02x}", b)).collect())
    }

    pub fn sign(&self, epoch: u64, msg: &[u8]) -> Vec<u8> {
        self.key(epoch).sign(msg).to_bytes().to_vec()
    }
}

/// Anyone can verify: a handle is the public key. Fails closed on any malformed input.
pub fn verify(who: &Handle, msg: &[u8], sig: &[u8]) -> bool {
    let hex = who.0.as_bytes();
    if hex.len() != 64 {
        return false;
    }
    let mut pk = [0u8; 32];
    for (i, c) in hex.chunks(2).enumerate() {
        match std::str::from_utf8(c).ok().and_then(|x| u8::from_str_radix(x, 16).ok()) {
            Some(b) => pk[i] = b,
            None => return false,
        }
    }
    let (Ok(vk), Ok(sig)) = (VerifyingKey::from_bytes(&pk), Signature::from_slice(sig)) else {
        return false;
    };
    vk.verify(msg, &sig).is_ok()
}

#[derive(Clone, Debug)]
pub struct Receipt {
    pub giver: Handle,
    pub receiver: Handle,
    /// The act the receipt covers, so a signature cannot be reused for a different favor.
    pub category: Category,
    /// Ids of the offer and need signals this act answered.
    pub offer_id: u64,
    pub need_id: u64,
    pub at: SystemTime,
    /// Distinguishes two otherwise identical acts. Chosen by the caller; must be unique per act.
    pub nonce: u64,
    pub giver_sig: Vec<u8>,
    pub receiver_sig: Vec<u8>,
}

/// Everything the signatures cover. Changing any field invalidates both signatures.
fn body(r: &Receipt) -> Vec<u8> {
    let secs = r.at.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!(
        "{}->{}|{:?}|{}|{}|{}|{}",
        r.giver.0, r.receiver.0, r.category, r.offer_id, r.need_id, secs, r.nonce
    )
    .into_bytes()
}

impl Receipt {
    /// Stable identity for replay checks: same act, same id.
    pub fn id(&self) -> Vec<u8> {
        body(self)
    }
}

impl Receipt {
    /// An unsigned receipt. Each side signs separately with its own key.
    pub fn new(
        giver: Handle,
        receiver: Handle,
        category: Category,
        offer_id: u64,
        need_id: u64,
        at: SystemTime,
        nonce: u64,
    ) -> Receipt {
        Receipt { giver, receiver, category, offer_id, need_id, at, nonce, giver_sig: vec![], receiver_sig: vec![] }
    }

    pub fn sign_as_giver(&mut self, id: &Identity, epoch: u64) {
        self.giver_sig = id.sign(epoch, &body(self));
    }

    pub fn sign_as_receiver(&mut self, id: &Identity, epoch: u64) {
        self.receiver_sig = id.sign(epoch, &body(self));
    }
}

/// Fail closed: both signatures must verify over the full body, and giver != receiver.
pub fn valid(r: &Receipt) -> bool {
    let b = body(r);
    r.giver != r.receiver && verify(&r.giver, &b, &r.giver_sig) && verify(&r.receiver, &b, &r.receiver_sig)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::time::Duration;

    pub fn id(n: u8) -> Identity { Identity::from_secret([n; 32]) }

    /// Test helper: a receipt signed by both identities in epoch 0.
    pub fn signed(g: &Identity, r: &Identity, at: SystemTime, nonce: u64) -> Receipt {
        let mut x = Receipt::new(g.handle(0), r.handle(0), Category::Carry, 1, 2, at, nonce);
        x.sign_as_giver(g, 0);
        x.sign_as_receiver(r, 0);
        x
    }

    fn mk(nonce: u64) -> Receipt {
        signed(&id(1), &id(2), SystemTime::UNIX_EPOCH + Duration::from_secs(1000), nonce)
    }

    #[test]
    fn good_receipt_is_valid() {
        assert!(valid(&mk(1)));
    }

    #[test]
    fn tampering_with_any_bound_field_fails() {
        let mut r = mk(1); r.category = Category::Meal; assert!(!valid(&r));
        let mut r = mk(1); r.at += Duration::from_secs(1); assert!(!valid(&r));
        let mut r = mk(1); r.need_id = 9; assert!(!valid(&r));
        let mut r = mk(1); r.nonce = 2; assert!(!valid(&r));
    }

    #[test]
    fn self_dealing_missing_and_forged_signatures_fail() {
        let a = id(1);
        assert!(!valid(&signed(&a, &a, SystemTime::UNIX_EPOCH, 1)));
        let mut r = mk(1); r.receiver_sig.clear(); assert!(!valid(&r));
        // receiver signature made by someone else's key
        let mut r = mk(1); r.sign_as_receiver(&id(3), 0); assert!(!valid(&r));
        // a handle that is not a public key
        let mut r = mk(1); r.giver = Handle("ann".into()); assert!(!valid(&r));
    }

    #[test]
    fn handles_rotate_and_are_stable() {
        let a = id(1);
        assert_ne!(a.handle(0), a.handle(1));
        assert_eq!(a.handle(5), a.handle(5));
        assert_ne!(id(1).handle(0), id(2).handle(0));
    }
}
