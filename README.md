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

Build status: `cargo build` and `cargo run` succeed on Rust 1.99 with two dead-code warnings
(`MemStore` and `Poker::new_day` unused in the binary).

## CLI

`pokkecon demo` runs the simulated loop. The other subcommands act on a real
(per-device) store: `offer <cat> <cell>` and `need <cat> <cell>` publish a
signal, `list` shows live signals, `match` and `poke` show what the matcher
would do. Signals persist in ~/.pokkecon/signals.jsonl (POKKECON_DIR overrides)
and identity in ~/.pokkecon/identity.key (prototype-grade, not the final key
storage design). Poke, decline and standing state is per-process for now. The demo prints one match, two pokes, a standing score, then a decline that blocks re-poking the pair without touching standing.

Implemented identity model: a handle is an ed25519 public key derived per day from a device-held
secret, so handles rotate and are not linkable without the secret. Receipts are signed separately by
giver and receiver. Not yet done: key storage, handle rotation inside live signals, and any
protection against a router correlating handles by timing or location cell.
