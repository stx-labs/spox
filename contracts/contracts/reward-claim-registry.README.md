# Reward Claim Registry

Permissionless keeper contract that registers PoX-5 stakers for automated reward claims via their signer-manager. Anyone (the staker, a pool operator, or an admin) may `register-for-claims` or `register-many-for-claims`; `tx-sender` is stored as `sponsor`. They choose a `start-reward-cycle`, whether to claim at most once or twice per reward cycle, and how many claim installments they want. spox later calls `process-reward-claims` to pull rewards from pox-5 when needed, claim for the staker, and advance the schedule. Each advance burns one installment from escrow. L1 sBTC withdrawals are tracked and settled separately.

## Invariants

- **Pending gate.** No claim runs unless there are a positive number registered `remaining-claims`, `next-claim-distribution` is less than `current-distribution-cycle`, and pox-5 `calculate-rewards` has covered that claim distribution.
- **Cadence is chosen at registration.**
- **Start cycle is explicit.** `start-reward-cycle` must be greater than or equal to the staker's `first-reward-cycle`.
- **Catch-up is allowed.** When many distributions are already past, keepers may call `process-reward-claim` repeatedly.
- **Always advance.** A failed `claim-rewards` or `claim-staker-rewards` still decrements `remaining-claims`, burns one installment from escrow, and advances the schedule. A junk or already-settled `withdrawal-request` is not stored and does not stall the claim.
- **Self-heal pull.** If a signer manager has earned rewards in pox-5 for the reward cycle, the registry calls `claim-rewards` before `claim-staker-rewards`.
- **Escrow then burn.** Fees are held as `prepaid-ustx` and burned one installment at a time when a claim is made. Admins do not need to escrow funds.
- **Anyone may register.** `register-for-claims` and `register-many-for-claims` have no caller restriction beyond live pox-5 positions under that signer-manager. `tx-sender` is stored as `sponsor` (so a helper contract can batch-register while the signed origin remains the sponsor).
- **One sponsor per registration.** Only that `sponsor` (as `tx-sender`) may `add-claims`. Cancel is the staker or the sponsor (matched as `tx-sender`); remaining `prepaid-ustx` is refunded to the sponsor.
- **add-claims preserves schedule.** Buying more installments for `{staker, signer-manager}` only increases `remaining-claims` and `prepaid-ustx`. Re-registering the same key fails with `ERR_ALREADY_REGISTERED`.
- **Batch registration / cancel are best-effort.** Failed entries are skipped without aborting the batch.
- **Max processed distribution.** On each claim the registry stores maximum distribution cycle processed by the registry. This makes registering under a different signer-manager easier for users, since they do not need to remember the reward cycle that they last processed.

## Gotchas

- Registration requires a **live** pox-5 stake under that signer-manager; bond-index is looked up, not passed.
- Empty / failed claims still burn a claim installment from escrow.
- The staker or the sponsor may `cancel-registration` / `cancel-many-registrations`. Remaining `prepaid-ustx` is refunded to the sponsor, not necessarily the staker. Pending L1 withdrawals remain settleable.
- Live sBTC-pending requests are indexed one map row per `{staker, signer-manager, request-id}` regardless of who created the withdrawal. `get-pending-withdrawals` lists a row after 7 Bitcoin blocks and only if sBTC status indicates acceptance or rejection from the sbtc signer. `settle-pending-withdrawal` drops the ID once sBTC has resolved (or the request is missing), even if the signer-manager errors.
- `get-pending-claims` and `get-pending-withdrawals` may return an empty `rows` list while `next` is still set. Paginate with `next` until it is `none`.
