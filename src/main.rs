mod attest;
mod matcher;
mod model;
mod poke;
mod standing;
mod store;

use attest::*;
use model::*;
use store::*;
use std::time::SystemTime;

fn h(s: &str) -> Handle {
    Handle(s.to_string())
}

fn main() {
    let now = SystemTime::now();
    let exp = now + SIGNAL_TTL;
    let cell = Cell("8f2f5a".into());

    let mut store = MemStore::default();
    store.put(Signal { id: 1, who: h("ann"), kind: Kind::Offer, category: Category::Carry, cell: cell.clone(), expires: exp });
    store.put(Signal { id: 2, who: h("bo"), kind: Kind::Need, category: Category::Carry, cell: cell.clone(), expires: exp });

    let mut standing = standing::Standing::default();
    let mut poker = poke::Poker::default();
    let signer = ToySigner;

    // observe -> match
    let matches = matcher::run(&store.all(), &standing, now);
    println!("matches: {}", matches.len());

    for m in &matches {
        // poke
        for p in poker.poke(m) {
            println!("poke {} : {}", p.to.0, p.text);
        }
        // act happens offline, then attest
        let r = sign_receipt(&signer, &m.offer.who, &m.need.who, now);
        if valid(&signer, &r) {
            standing.record(&r);
        }
    }

    println!("ann standing: {:.2}", standing.score(&h("ann"), now));
}
