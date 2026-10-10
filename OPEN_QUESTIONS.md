# Open questions

1. Sybil and collusion: two phones trading fake attestations farm standing.
   Candidates: weight attestations by the attester's own standing and by graph
   distance; cap standing gain per counterparty per week; require a third-party witness for high-value acts.
2. Who runs the router? One operator (simple, a point of control) or many
   federated relays (closer to the story, harder to build).
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
