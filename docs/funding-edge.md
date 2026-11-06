# Cashu at the funding edge (DRAFT design note)

Status: design note; nothing implemented, no payments code.

Role (decided product direction): Cashu is Pokkecon's funding rail. It
funds bounties and public goods around the favor loop; favors and
standing stay money-free. Concrete mechanisms are unresolved design
questions, not approved design.

## The rule

The favor loop - observe, match, poke, act, attest, standing - stays
money-free. Standing remains local, decaying, non-spendable, peer-
established. The moment a favor has a price the product becomes a
marketplace and the standing mechanism dies.

## What Cashu is for here

Funding, not favors. The framing: "like a next gen Gitcoin" - Gitcoin
routed money to public goods; Pokkecon routes attention and small acts,
and Cashu lets funders put real sats behind what they want more of. How
pools are funded, held, and paid out, and how they are advertised, is
undesigned.

## Hard boundaries (proposals, not settled policy)

- No pokkecon-run mint (proposed): custody and issuance raise
  money-transmitter questions this project would not take on. Existing
  mints issue the tokens; pokkecon would only route them.
- No payments code without a separate explicit decision. This note
  authorizes nothing by itself.
- Favors never carry ecash. A poke with money attached is a marketplace
  order, not a poke.

## Open questions

- All mechanism design: pool funding, custody, payout conditions,
  expiry, anti-gaming, advertising.
- nonfungible.cash for attestation receipts stays unexplored.
