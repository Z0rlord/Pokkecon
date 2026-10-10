mod attest;
mod matcher;
mod model;
mod poke;
mod standing;
mod store;

use attest::*;
use model::*;
use standing::Standing;
use store::*;
use std::io;
use std::path::PathBuf;
use std::time::SystemTime;

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => usage(),
        Some("demo") => demo(),
        Some("offer") => publish(Kind::Offer, &args[1..]),
        Some("need") => publish(Kind::Need, &args[1..]),
        Some("list") => list(),
        Some("match") => show_matches(),
        Some("poke") => show_pokes(),
        Some("decline") => decline(&args[1..]),
        Some(other) => {
            eprintln!("unknown command: {}", other);
            usage()
        }
    }
}

fn usage() -> io::Result<()> {
    println!(
        "pokkecon demo                 run the end-to-end simulation
pokkecon offer <cat> <cell>   publish an offer signal (1h TTL)
pokkecon need <cat> <cell>    publish a need signal (1h TTL)
pokkecon list                 show live signals
pokkecon match                show current matches
pokkecon poke                 show pokes the matcher would send
pokkecon decline <oid> <nid>  decline the match for offer <oid> and need <nid>

categories: carry lend guide meal repair
signals persist in $POKKECON_DIR/signals.jsonl (default ~/.pokkecon/).
Receipts and poke/decline state persist too (receipts.jsonl, poker.json);
standing is rebuilt from the receipt log on every run. Identity:
$POKKECON_DIR/identity.key, created on first run - prototype-grade key
storage, not the final design."
    );
    Ok(())
}

fn data_dir() -> PathBuf {
    match std::env::var("POKKECON_DIR") {
        Ok(d) => PathBuf::from(d),
        Err(_) => PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".pokkecon"),
    }
}

/// Load the identity secret, creating it on first run. Prototype-grade key
/// storage: a hex file with owner-only permissions. Rotation, hardware
/// backing and loss recovery are all undecided; see OPEN_QUESTIONS.md.
fn identity() -> io::Result<Identity> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("identity.key");
    if let Ok(hex) = std::fs::read_to_string(&path) {
        let bytes = from_hex(hex.trim()).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "identity.key is not 32 hex bytes")
        })?;
        return Ok(Identity::from_secret(bytes));
    }
    // No rand crate yet; the OS CSPRNG is enough for a prototype secret.
    let mut secret = [0u8; 32];
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(&mut secret)?;
    std::fs::write(&path, to_hex(&secret))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(Identity::from_secret(secret))
}

fn store_path() -> PathBuf {
    data_dir().join("signals.jsonl")
}

fn receipts_path() -> PathBuf {
    data_dir().join("receipts.jsonl")
}

fn poker_path() -> PathBuf {
    data_dir().join("poker.json")
}

fn publish(kind: Kind, args: &[String]) -> io::Result<()> {
    if args.len() != 2 {
        eprintln!("usage: pokkecon {} <category> <cell>", if kind == Kind::Offer { "offer" } else { "need" });
        return usage();
    }
    let category = Category::from_name(&args[0]).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, format!("unknown category: {}", args[0]))
    })?;
    let now = SystemTime::now();
    let me = identity()?.handle(epoch_of(now));
    let mut store = FileStore::open(store_path())?;
    let id = store.all().iter().map(|s| s.id).max().unwrap_or(0) + 1;
    store.put(Signal {
        id,
        who: me,
        kind,
        category: category.clone(),
        cell: Cell(args[1].clone()),
        expires: now + SIGNAL_TTL,
    })?;
    println!("published {:?} #{}: {} in {} for 1h", kind, id, category.name(), args[1]);
    Ok(())
}

fn list() -> io::Result<()> {
    let now = SystemTime::now();
    let store = FileStore::open(store_path())?;
    let mut n = 0;
    for s in store.all().iter().filter(|s| s.live(now)) {
        println!("#{} {:?} {} in {} by {}", s.id, s.kind, s.category.name(), s.cell.0, &s.who.0[..8]);
        n += 1;
    }
    println!("{} live signal(s)", n);
    Ok(())
}

fn matched(now: SystemTime) -> io::Result<Vec<matcher::Match>> {
    let store = FileStore::open(store_path())?;
    // Standing is rebuilt from the device's own receipt log on every run.
    let receipts = ReceiptStore::open(receipts_path())?;
    let standing = Standing::from_receipts(receipts.all(), now);
    Ok(matcher::run(&store.all(), &standing, now))
}

fn decline(args: &[String]) -> io::Result<()> {
    if args.len() != 2 {
        eprintln!("usage: pokkecon decline <offer_id> <need_id>");
        return usage();
    }
    let (oid, nid) = (args[0].parse::<u64>(), args[1].parse::<u64>());
    let (Ok(oid), Ok(nid)) = (oid, nid) else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "signal ids must be numbers"));
    };
    let now = SystemTime::now();
    let store = FileStore::open(store_path())?;
    let all = store.all();
    let offer = all.iter().find(|s| s.id == oid && s.kind == Kind::Offer && s.live(now));
    let need = all.iter().find(|s| s.id == nid && s.kind == Kind::Need && s.live(now));
    let (Some(offer), Some(need)) = (offer, need) else {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no such live offer/need pair"));
    };
    let m = matcher::Match { offer: offer.clone(), need: need.clone() };
    let me = identity()?.handle(epoch_of(now));
    let path = poker_path();
    let mut poker = poke::Poker::load(&path)?;
    if !poker.decline(&m, &me, now) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "not your match to decline"));
    }
    poker.save(&path)?;
    println!("declined; this pair will not be re-poked for 24h. Standing is untouched.");
    Ok(())
}

fn show_matches() -> io::Result<()> {
    let ms = matched(SystemTime::now())?;
    for m in &ms {
        println!(
            "match: offer #{} ({}) x need #{} ({}) in {}",
            m.offer.id, &m.offer.who.0[..8], m.need.id, &m.need.who.0[..8], m.offer.cell.0
        );
    }
    println!("{} match(es)", ms.len());
    Ok(())
}

fn show_pokes() -> io::Result<()> {
    let now = SystemTime::now();
    let path = poker_path();
    let mut poker = poke::Poker::load(&path)?;
    let mut n = 0;
    for m in &matched(now)? {
        for p in poker.poke_at(m, now) {
            println!("poke {} : {}", &p.to.0[..8], p.text);
            n += 1;
        }
    }
    poker.save(&path)?;
    println!("{} poke(s)", n);
    Ok(())
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn from_hex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

fn demo() -> io::Result<()> {
    let now = SystemTime::now();
    let exp = now + SIGNAL_TTL;
    let cell = Cell("8f2f5a".into());
    let epoch = epoch_of(now);

    // Demo identities. Real ones would be random secrets kept on the device.
    let ann = Identity::from_secret([1; 32]);
    let bo = Identity::from_secret([2; 32]);
    let (ha, hb) = (ann.handle(epoch), bo.handle(epoch));

    let path = std::env::temp_dir().join(format!("pokkecon-demo-{}.jsonl", std::process::id()));
    let mut store = FileStore::open(&path)?;
    store.put(Signal { id: 1, who: ha.clone(), kind: Kind::Offer, category: Category::Carry, cell: cell.clone(), expires: exp })?;
    store.put(Signal { id: 2, who: hb.clone(), kind: Kind::Need, category: Category::Carry, cell: cell.clone(), expires: exp })?;
    println!("signals persisted to {}", path.display());

    let mut standing = Standing::default();
    let mut poker = poke::Poker::default();
    let rpath = std::env::temp_dir().join(format!("pokkecon-demo-receipts-{}.jsonl", std::process::id()));
    let mut rstore = ReceiptStore::open(&rpath)?;

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
            match standing.record(&r, now) {
                Err(e) => println!("receipt rejected: {:?}", e),
                Ok(()) => rstore.put(r.clone())?,
            }
        }
    }

    println!("ann standing: {:.2}", standing.score(&ha, now));

    // Standing is rebuilt from the receipt log on every start; the device's
    // own score links its handles across rotations, locally and only locally.
    let rebuilt = Standing::from_receipts(rstore.all(), now);
    println!("standing rebuilt from log: {:.2}", rebuilt.score(&ha, now));
    println!("ann own score across rotations: {:.2}", rebuilt.own_score(&ann, now));

    // A decline: free for the decliner, and the pair is not re-poked for a cooldown.
    let mut poker2 = poke::Poker::default();
    if let Some(m) = matcher::run(&store.all(), &standing, now).first() {
        let before = standing.score(&ha, now);
        println!("re-poke fires: {}", !poker2.poke_at(m, now).is_empty());
        poker2.decline(m, &hb, now);
        println!("re-poke after decline fires: {}", !poker2.poke_at(m, now).is_empty());
        println!("ann standing unchanged by bo's decline: {}", standing.score(&ha, now) == before);
    }

    // Reopen: the signals survive the process that wrote them.
    let reread = FileStore::open(&path)?;
    println!("signals after reopen: {}", reread.all().len());

    // The day rolls over: each publisher re-asserts its live signal under the
    // new day's handle. Only the secret holder can do it; the router cannot
    // tell yesterday's and today's handles belong to the same person.
    let next = epoch + 1;
    let rotated: Vec<Signal> = store
        .all()
        .iter()
        .map(|s| {
            if s.who == ha {
                rehandle(s, &ann, next)
            } else if s.who == hb {
                rehandle(s, &bo, next)
            } else {
                s.clone()
            }
        })
        .collect();
    println!("signals rehandled for new epoch: {}", rotated.iter().all(|s| s.who != ha && s.who != hb));

    // A receipt mixing epochs (old handle, new key) fails closed; one signed
    // consistently in the new epoch verifies.
    let mut stale = Receipt::new(ha.clone(), bo.handle(next), Category::Carry, 1, 2, now, 2);
    stale.sign_as_giver(&ann, next);
    stale.sign_as_receiver(&bo, next);
    println!("cross-epoch receipt rejected: {}", !valid(&stale));
    let mut fresh = Receipt::new(ann.handle(next), bo.handle(next), Category::Carry, 1, 2, now, 3);
    fresh.sign_as_giver(&ann, next);
    fresh.sign_as_receiver(&bo, next);
    println!("post-rotation receipt valid: {}", valid(&fresh));

    std::fs::remove_file(&path).ok();
    std::fs::remove_file(&rpath).ok();
    Ok(())
}
