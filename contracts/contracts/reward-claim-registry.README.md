# Reward Claim Registry

Permissionless keeper contract that registers PoX-5 stakers for automated reward claims via their signer-manager. A staker (or an admin on their behalf) buys claim installments by escrowing STX, choosing a `start-reward-cycle` and whether to claim at most once or twice per reward cycle (`one-claim-per-reward-cycle`). spox later calls `process-reward-claim(s)` to pull rewards from pox-5 when needed, claim for the staker, and advance the schedule. Each advance burns one installment from escrow. L1 sBTC withdrawals are tracked and settled separately.

## Invariants

- **Pending gate.** No claim runs unless there are a positive number registered `remaining-cycles`, `next-claim-distribution` is less than `current-distribution-cycle`, and pox-5 `calculate-rewards` has covered that claim distribution.
- **Cadence is chosen at registration.**
- **Start cycle is explicit.** `start-reward-cycle` must be greater than or equal to the staker's `first-reward-cycle`.
- **Catch-up is allowed.** When many distributions are already past, keepers may call `process-reward-claim` repeatedly.
- **Always advance.** A failed `claim-rewards` or `claim-staker-rewards` still decrements `remaining-cycles`, burns one installment from escrow, and advances the schedule. A junk or already-settled `withdrawal-request` is not stored and does not stall the claim.
- **Self-heal pull.** If a signer manager has earned rewards in pox-5 for the reward cycle, the registry calls `claim-rewards` before `claim-staker-rewards`.
- **Escrow then burn.** Fees are held as `prepaid-ustx` and burned one installment at a time when a claim is made. Admins do not need to escrow funds.
- **Self-register (or admin).** Only the staker or an admin may `register-for-claims` / `add-claims`. Cancel is staker-only, including when an admin created the registration; remaining escrow is refunded to the staker.
- **add-claims preserves schedule.** Buying more installments for `{staker, signer-manager}` only increases `remaining-cycles` and `prepaid-ustx`. Re-registering the same key fails with `ERR_ALREADY_REGISTERED`.

## Gotchas

- Registration requires a **live** pox-5 stake under that signer-manager; bond-index is looked up, not passed.
- Empty / failed claims still burn a claim installment from escrow.
- Only the staker may `cancel-registration` (admins cannot cancel for them). Remaining `prepaid-ustx` is refunded to the staker; pending L1 withdrawals remain settleable.
- Live sBTC-pending requests are indexed one map row per `{staker, signer-manager, request-id}` regardless of who created the withdrawal. `get-pending-withdrawals` lists a row after 7 Bitcoin blocks and only if sBTC status indicates acceptance or rejection from the sbtc signer. `settle-pending-withdrawal` drops the ID once sBTC has resolved (or the request is missing), even if the signer-manager errors.
- `get-pending-claims` and `get-pending-withdrawals` may return an empty `rows` list while `next` is still set. Paginate with `next` until it is `none`.
