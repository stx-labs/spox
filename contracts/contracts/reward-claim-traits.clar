;; Trait the reward-claim-registry dispatches on to claim and settle rewards
;; for a staker via their signer-manager.

(define-trait reward-claim-signer-manager-trait
  (
    ;; Claim a staker's rewards for a reward cycle (and optional bond index).
    ;; Returns the net `earned` credited to the staker and, when the payout was
    ;; routed to an L1 sBTC withdrawal, the `withdrawal-request` (`none` for a
    ;; direct sBTC payout).
    (claim-staker-rewards
      (principal uint (optional uint))
      (response { earned: uint, withdrawal-request: (optional uint) } uint)
    )
    ;; Pull the signer's rewards for a reward cycle from pox-5 into the
    ;; signer-manager so per-staker claims can be paid. `bond-periods` are the
    ;; bond indices to pull alongside the STX-stake rewards (empty for STX only).
    ;; Must run once per (signer, reward-cycle, scope) before claim-staker-rewards
    ;; will pay; the reward-claim-registry calls it itself when pox-5 still shows
    ;; rewards owed. The return mirrors pox-5's claim-rewards and is unused by the
    ;; registry, but the type must match for trait conformance.
    (claim-rewards
      ((list 6 uint) uint)
      (response {
        stx-rewards: { earned: uint, rewards-per-token: uint },
        bond-rewards: (list 6 { earned: uint, bond-index: uint, rewards-per-token: uint }),
        bond-totals: uint,
        total-rewards: uint,
      } uint)
    )
    ;; Settle an accepted L1 withdrawal by its sbtc-registry request-id.
    (settle-accepted-withdrawal (uint) (response bool uint))
    ;; Reclaim a rejected L1 withdrawal back to the staker who earned it.
    (reclaim-failed-withdrawal (uint) (response bool uint))
  )
)
