use crate::model::Handle;
use std::time::SystemTime;

/// Replace with ed25519. This toy exists so the loop runs end to end.
pub trait Signer {
    fn sign(&self, who: &Handle, msg: &[u8]) -> Vec<u8>;
    fn verify(&self, who: &Handle, msg: &[u8], sig: &[u8]) -> bool;
}

pub struct ToySigner;

impl Signer for ToySigner {
    fn sign(&self, who: &Handle, msg: &[u8]) -> Vec<u8> {
        let mut v = who.0.as_bytes().to_vec();
        v.extend_from_slice(msg);
        v
    }
    fn verify(&self, who: &Handle, msg: &[u8], sig: &[u8]) -> bool {
        self.sign(who, msg) == sig
    }
}

#[derive(Clone, Debug)]
pub struct Receipt {
    pub giver: Handle,
    pub receiver: Handle,
    pub at: SystemTime,
    pub giver_sig: Vec<u8>,
    pub receiver_sig: Vec<u8>,
}

fn body(giver: &Handle, receiver: &Handle) -> Vec<u8> {
    format!("{}->{}", giver.0, receiver.0).into_bytes()
}

pub fn sign_receipt(
    s: &dyn Signer,
    giver: &Handle,
    receiver: &Handle,
    at: SystemTime,
) -> Receipt {
    let b = body(giver, receiver);
    Receipt {
        giver: giver.clone(),
        receiver: receiver.clone(),
        at,
        giver_sig: s.sign(giver, &b),
        receiver_sig: s.sign(receiver, &b),
    }
}

/// Fail closed: both signatures must verify, and giver != receiver.
pub fn valid(s: &dyn Signer, r: &Receipt) -> bool {
    let b = body(&r.giver, &r.receiver);
    r.giver != r.receiver
        && s.verify(&r.giver, &b, &r.giver_sig)
        && s.verify(&r.receiver, &b, &r.receiver_sig)
}
