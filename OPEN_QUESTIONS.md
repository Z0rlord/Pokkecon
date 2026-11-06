# Open questions

1. Sybil and collusion: two phones trading fake attestations farm standing.
   Done: per-pair weekly cap, attestation weighted by the attester's standing
   (base + capped bonus). Open: graph-distance weighting (needs a trust graph
   that does not yet exist), a third-party witness for high-value acts.
2. Transport: Nostr relays, selected 2026-11-05. Open: signal visibility -
   open ephemeral signals any client can match locally (no trusted router,
   but coarse cells are public) vs. signals encrypted to a matcher key
   (private, but a trusted matcher exists). Also open: who runs the relay(s)
   in practice, and push delivery to phones.
3. What is an "act"? Keep the catalog small and physical: carry, lend, guide, share a meal, small repair.
4. Safety: meeting strangers. Public places only by default, an opt-out per category, a block list.
5. Legal: gift framing, liability for in-person acts, data minimisation.
6. Cold start: needs a dense local pool. Start in one neighborhood or one community.
7. Secret loss and theft: standing and identity live in one device secret.
   Loss erases standing history; theft links the owner's past handles. Key
   backup, recovery and revocation are undesigned.
8. Standing vs rotation is resolved (local-only link, see README). Open: does
   anyone else ever need to verify a standing claim across rotations, e.g. a
   witness weighting per question 1, without re-linking handles?
9. Rotation linkage: rehandled signals keep their id (an observable cross-day
   link for anyone who recorded it), receipts name both parties, and
   day-rollover timing can correlate a person in sparse areas. A known limitation
   pending review; whether re-asserted intents should mint fresh ids is
   undesigned.
