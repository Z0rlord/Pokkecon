# pokkecon (scaffold)

A gift-economy router in the spirit of Bruce Sterling's "Maneki Neko" (1998).
People post small offers and needs. A matcher pairs them nearby, the app pokes
each side with one tiny assignment, both sides attest the act happened, and
standing accrues. No money, no central ledger of favors owed between two people.

## Core loop

observe -> match -> poke -> act -> attest -> standing

1. Observe: a user's phone publishes coarse signals: Offer or Need, a category, a geo cell, a time window.
2. Match: the matcher pairs complementary signals in the same cell and window.
3. Poke: each side gets one small, declinable assignment. A decline costs nothing.
4. Act: the stranger does the thing.
5. Attest: both parties sign a receipt. One-sided claims do not count.
6. Standing: attested acts raise a local standing score. Standing steers who is
   poked first. It is never shown as a leaderboard and never spent.

## Design rules (the part that makes it Maneki Neko and not a marketplace)

- No pairwise debt. Giver and receiver are rarely the same two people, so nobody is "owed".
- Coarse location only (cell, not coordinates). Signals expire fast.
- Pokes are rate limited per person per day. Declining is free and invisible to the other side.
- Standing decays. Old favors do not buy permanent privilege.
- The router sees signals, not identities. Handles are rotating pseudonyms.
- Fail closed: no valid signature, no scope, no match.

## Layout

- src/model.rs     signals, handles, cells
- src/matcher.rs   complementary-signal pairing
- src/standing.rs  decaying standing from attested acts
- src/poke.rs      assignments, rate limits, decline
- src/attest.rs    dual-signed receipts, Signer trait (stub)
- src/store.rs     Store trait + in-memory and JSON-lines file impls
- src/main.rs      CLI entry point (offer/need/list/match/poke) plus the simulation (`pokkecon demo`)

## Stubbed on purpose

- Network transport (relay, push to phones, Nostr or plain HTTP).
- Abuse handling beyond a per-pair cap (no graph-distance weighting, no witnesses, no sybil resistance).
  See OPEN_QUESTIONS.md.

Build status: `cargo build` and `cargo run` succeed on Rust 1.99 with one dead-code warning
(`MemStore` unused in the binary).

## CLI

`pokkecon demo` runs the simulated loop. The other subcommands act on a real
(per-device) store: `offer <cat> <cell>` and `need <cat> <cell>` publish a
signal, `list` shows live signals, `match` and `poke` show what the matcher
would do, `decline <oid> <nid>` declines a match. Everything persists under
~/.pokkecon/ (POKKECON_DIR overrides): signals.jsonl, receipts.jsonl,
poker.json (rate limits and declines), and identity.key (prototype-grade,
not the final key storage design). Standing is rebuilt from the receipt log
on every run. The demo prints one match, two pokes, a standing score, then a decline that blocks re-poking the pair without touching standing.

Implemented identity model: a handle is an ed25519 public key derived per day from a device-held
secret, so handles rotate and are not linkable without the secret. Receipts are signed separately by
giver and receiver. When the day rolls over, a publisher re-asserts each live signal under the
new day's handle (`rehandle`); only the secret holder can carry that continuity,
the router cannot infer it. Receipts signed before the rotation stay valid because
they verify against the handles bound inside them. Not yet done: real key storage,
and any protection against a router correlating handles by timing or location cell.

## The standing vs rotation tradeoff (accepted)

Handles rotate daily so the router cannot link a person across days. Standing
is only useful if it survives rotation. The accepted resolution: the device
keeps a local-only link from its own past handles to its master secret
(`Standing::own_score` derives it on the fly) and rebuilds standing from a
local receipt log on every start. This re-creates linkability on-device only.
The link never leaves the device, and everyone else's standing still resets
at each rotation because nobody else can do that linking. The rejected
alternative - standing resets daily for everyone, yourself included - guts
the mechanism: reputation would evaporate at midnight and the matcher would
have no history to rank offers with.

Consequences kept visible:

- A lost or wiped device loses its standing history with its secret.
- A stolen secret links the owner's past handles. Recovery and revocation are
  open questions (see OPEN_QUESTIONS.md).
