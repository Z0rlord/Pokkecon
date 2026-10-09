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
- src/store.rs     Store trait + in-memory impl
- src/main.rs      end-to-end simulation of the loop

## Stubbed on purpose

- Real signatures (Signer is a toy; swap in ed25519).
- Network transport (relay, push to phones, Nostr or plain HTTP).
- Persistence beyond memory.
- Abuse handling: sybil resistance, collusion rings farming standing.
  See OPEN_QUESTIONS.md.

Not compiled yet: written without a Rust toolchain on hand. Run `cargo run`
and expect to fix small errors.
