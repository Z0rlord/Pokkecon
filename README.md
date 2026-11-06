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
6. Standing: attested acts raise a local standing score. Each act is weighted by
   the attester's own standing at the time (base 0.25 so new users can start,
   bonus capped at 0.75 so no whale dominates), decays with age, and is capped
   per counterparty per week. A collusion ring of fresh phones can only vouch
   at base weight and grows slowly. Standing steers who is poked first. It is
   never shown as a leaderboard and never spent.

## Design rules (the part that makes it Maneki Neko and not a marketplace)

- No pairwise debt. Giver and receiver are rarely the same two people, so nobody is "owed".
- Coarse location only (cell, not coordinates). Signals expire fast.
- Pokes are rate limited per person per day. Declining is free and invisible to the other side.
- Standing decays. Old favors do not buy permanent privilege.
- The router sees signals, not identities. Handles are rotating pseudonyms.
- Funding is not favors: a Cashu ecash funding edge (bounties, public goods)
  is in design only; no favor ever carries money. See docs/funding-edge.md.
- Fail closed: no valid signature, no scope, no match.

## Layout

- src/model.rs     signals, handles, cells
- src/matcher.rs   complementary-signal pairing
- src/standing.rs  decaying standing from attested acts
- src/poke.rs      assignments, rate limits, decline
- src/attest.rs    dual-signed receipts, rotating handles
- src/nostr.rs     Nostr wire layer: events, NIP-44 encryption, NIP-59 gift wrap (crypto via the rust-nostr crate)
- src/relay.rs     relay protocol, sync client, local test relay
- src/transport.rs handles <-> Nostr transport keys, signal/poke packaging
- src/nostr_vectors.rs  official NIP-44 vector and interop regression tests
- src/store.rs     Store trait + in-memory and JSON-lines file impls
- src/main.rs      CLI entry point (offer/need/list/match/poke) plus the simulation (`pokkecon demo`)

## Stubbed on purpose

- Push delivery to phones and any relay deployment.
- Abuse handling beyond attester-weighted standing and a per-pair cap (no witnesses yet, no
  graph-distance weighting). See OPEN_QUESTIONS.md.

Build status: `cargo build`, `cargo test` (48 tests) and `cargo run` succeed on Rust 1.99.

## Transport: Nostr (in progress)

What exists today:

- Signals are packaged as ephemeral Nostr events (kind 20001) that relays are
  not expected to store, matching the 1-hour TTL. A NIP-40 expiration tag
  mirrors the TTL; enforcement stays local.
- Pokes and receipts travel as NIP-59 gift-wrapped private messages: the outer
  event is a one-time key, so a relay sees the recipient and a time window,
  not the sender.
- Each device derives a secp256k1 transport key per day from the same secret
  as its handles. Every signal and poke carries a signature proving the
  transport key belongs to the claimed handle for that day, so counterparties
  verify senders without a central registry.
- The relay protocol, a synchronous client and a local test relay are
  included. All tests run against a loopback relay; no public relay is
  contacted and no deployment exists.

Honest limits:

- Who may read signals is an open product decision: open ephemeral signals
  that any client can match, or signals encrypted to a matcher key. Nothing
  is broadcast today either way.
- Cryptography is delegated to the rust-nostr crate (v0.45.6), which passes
  the protocol's official test vectors in this repo's test suite. No
  independent security audit covering this crate version has been verified;
  the 2023 Cure53 audit covered the NIP-44 spec and different reference
  implementations.
- Daily rotation alone does not make a person unlinkable across days:
  republished signals keep their id, receipts name both parties, and
  same-place same-time patterns at day rollover can correlate.

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
secret, so handles rotate and one day's handle cannot be derived from another without the secret. Receipts are signed separately by
giver and receiver. When the day rolls over, a publisher re-asserts each live signal under the
new day's handle (`rehandle`). Only the secret holder can do this - but the
re-asserted signal keeps its id, so an observer who recorded it under last
day's handle can link the two. Rotation prevents deriving one handle from
another; it does not erase that correlation channel. Receipts signed before the rotation stay valid because
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
