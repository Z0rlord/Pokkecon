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

fn main() {
    let now = SystemTime::now();
    let exp = now + SIGNAL_TTL;
    let cell = Cell("8f2f5a".into());
    let epoch = epoch_of(now);

    // Demo identities. Real ones would be random secrets kept on the device.
    let ann = Identity::from_secret([1; 32]);
    let bo = Identity::from_secret([2; 32]);
    let (ha, hb) = (ann.handle(epoch), bo.handle(epoch));

    let mut store = MemStore::default();
    store.put(Signal { id: 1, who: ha.clone(), kind: Kind::Offer, category: Category::Carry, cell: cell.clone(), expires: exp });
    store.put(Signal { id: 2, who: hb.clone(), kind: Kind::Need, category: Category::Carry, cell: cell.clone(), expires: exp });

    let mut standing = standing::Standing::default();
    let mut poker = poke::Poker::default();

    // observe -> match
    let matches = matcher::run(&store.all(), &standing, now);
    println!("matches: {}", matches.len());

    for m in &matches {
        // poke
        for p in poker.poke(m) {
            println!("poke {} : {}", &p.to.0[..8], p.text);
        }
        // act happens offline; then each side signs the same receipt with its own key
        let mut r = Receipt::new(m.offer.who.clone(), m.need.who.clone(), m.need.category.clone(), m.offer.id, m.need.id, now, 1);
        r.sign_as_giver(&ann, epoch);
        r.sign_as_receiver(&bo, epoch);
        if valid(&r) {
            if let Err(e) = standing.record(&r, now) {
                println!("receipt rejected: {:?}", e);
            }
        }
    }

    println!("ann standing: {:.2}", standing.score(&ha, now));

    // A decline: free for the decliner, and the pair is not re-poked for a cooldown.
    let mut poker2 = poke::Poker::default();
    if let Some(m) = matcher::run(&store.all(), &standing, now).first() {
        let before = standing.score(&ha, now);
        println!("re-poke fires: {}", !poker2.poke_at(m, now).is_empty());
        poker2.decline(m, &hb, now);
        println!("re-poke after decline fires: {}", !poker2.poke_at(m, now).is_empty());
        println!("ann standing unchanged by bo's decline: {}", standing.score(&ha, now) == before);
    }
}
