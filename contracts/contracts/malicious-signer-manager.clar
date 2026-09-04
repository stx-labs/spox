;; Test signer-manager that re-enters reward-claim-registry from trait
;; callbacks. Trait-typed reentry uses mock-signer-manager (already
;; published) so this contract does not reference itself. The registry
;; guard is a single contract-wide flag, so the inner call still hits
;; ERR_REENTRANT_CALL before dispatching on that trait.

(impl-trait .reward-claim-registry.reward-claim-signer-manager-trait)
(impl-trait 'ST000000000000000000002AMW42H.pox-5.signer-manager-trait)
(use-trait signer-manager-trait 'ST000000000000000000002AMW42H.pox-5.signer-manager-trait)

(define-constant REENTER_NONE u0)
(define-constant REENTER_PROCESS_CLAIM u1)
(define-constant REENTER_PROCESS_CLAIMS u2)
(define-constant REENTER_CANCEL u3)
(define-constant REENTER_SETTLE u4)
(define-constant REENTER_ADD_CLAIMS u5)
(define-constant REENTER_REGISTER u6)

(define-data-var reenter-mode uint REENTER_NONE)
(define-data-var reenter-staker principal tx-sender)
(define-data-var last-reenter-error (optional uint) none)
(define-data-var withdrawal-id (optional uint) none)

;; pox-5 callback: always accept the stake.
;; #[allow(unnecessary_public)]
(define-public (validate-stake!
    ;; #[allow(unused_binding)]
    (staker principal)
    ;; #[allow(unused_binding)]
    (first-index uint)
    ;; #[allow(unused_binding)]
    (num-indexes uint)
    ;; #[allow(unused_binding)]
    (amount-ustx uint)
    ;; #[allow(unused_binding)]
    (amount-sats uint)
    ;; #[allow(unused_binding)]
    (is-bond bool)
    ;; #[allow(unused_binding)]
    (signer-calldata (optional (buff 500)))
  )
  (ok true)
)

(define-public (register-self
    (signer-manager <signer-manager-trait>)
    (signer-key (buff 33))
    (auth-id uint)
    (signer-sig (buff 65))
  )
  (begin
    (try! (contract-call? 'ST000000000000000000002AMW42H.pox-5 grant-signer-key
      signer-key current-contract auth-id signer-sig
    ))
    (contract-call? 'ST000000000000000000002AMW42H.pox-5 register-signer
      signer-manager signer-key
    )
  )
)

;; #[allow(unchecked_data)]
(define-private (note-reenter-err (err-code uint))
  (var-set last-reenter-error (some err-code))
)

(define-private (attempt-reenter)
  (if (is-eq (var-get reenter-mode) REENTER_NONE)
    true
    (begin
      (var-set last-reenter-error none)
      (if (is-eq (var-get reenter-mode) REENTER_PROCESS_CLAIM)
        (match (contract-call? .reward-claim-registry process-reward-claim
            (var-get reenter-staker)
            .mock-signer-manager
          )
          ok-v (var-set last-reenter-error none)
          err-v (note-reenter-err err-v)
        )
        (if (is-eq (var-get reenter-mode) REENTER_PROCESS_CLAIMS)
          (match (contract-call? .reward-claim-registry process-reward-claims
              .mock-signer-manager
              (list (var-get reenter-staker))
            )
            ok-v (var-set last-reenter-error none)
            err-v (note-reenter-err err-v)
          )
          (if (is-eq (var-get reenter-mode) REENTER_CANCEL)
            (match (contract-call? .reward-claim-registry cancel-registration
                (var-get reenter-staker)
                current-contract
              )
              ok-v (var-set last-reenter-error none)
              err-v (note-reenter-err err-v)
            )
            (if (is-eq (var-get reenter-mode) REENTER_SETTLE)
              (match (contract-call? .reward-claim-registry settle-pending-withdrawals
                  .mock-signer-manager
                  (list {
                    staker: (var-get reenter-staker),
                    request-id: u1,
                  })
                )
                ok-v (var-set last-reenter-error none)
                err-v (note-reenter-err err-v)
              )
              (if (is-eq (var-get reenter-mode) REENTER_ADD_CLAIMS)
                (match (contract-call? .reward-claim-registry add-claims
                    (var-get reenter-staker)
                    current-contract
                    u10
                  )
                  ok-v (var-set last-reenter-error none)
                  err-v (note-reenter-err err-v)
                )
                (if (is-eq (var-get reenter-mode) REENTER_REGISTER)
                  (match (contract-call? .reward-claim-registry register-for-claims
                      (var-get reenter-staker)
                      .mock-signer-manager
                      u1
                      true
                      u10
                    )
                    ok-v (var-set last-reenter-error none)
                    err-v (note-reenter-err err-v)
                  )
                  true
                )
              )
            )
          )
        )
      )
      true
    )
  )
)

;; #[allow(unnecessary_public)]
(define-public (claim-staker-rewards
    ;; #[allow(unused_binding)]
    (staker principal)
    ;; #[allow(unused_binding)]
    (reward-cycle uint)
    ;; #[allow(unused_binding)]
    (bond-index (optional uint))
  )
  (begin
    (attempt-reenter)
    (ok {
      earned: u0,
      withdrawal-request: (var-get withdrawal-id),
    })
  )
)

;; #[allow(unnecessary_public)]
(define-public (claim-rewards
    ;; #[allow(unused_binding)]
    (bond-periods (list 6 uint))
    ;; #[allow(unused_binding)]
    (reward-cycle uint)
  )
  (begin
    (attempt-reenter)
    (ok {
      stx-rewards: { earned: u0, rewards-per-token: u0 },
      bond-rewards: (list { earned: u0, bond-index: u0, rewards-per-token: u0 }),
      bond-totals: u0,
      total-rewards: u0,
    })
  )
)

;; #[allow(unnecessary_public)]
(define-public (settle-accepted-withdrawal
  ;; #[allow(unused_binding)]
  (request-id uint)
)
  (begin
    (attempt-reenter)
    (ok true)
  )
)

;; #[allow(unnecessary_public)]
(define-public (reclaim-failed-withdrawal
  ;; #[allow(unused_binding)]
  (request-id uint)
)
  (begin
    (attempt-reenter)
    (ok true)
  )
)

(define-public (set-reenter-mode
    (mode uint)
    (staker principal)
  )
  (begin
    ;; #[allow(unchecked_data)]
    (var-set reenter-mode mode)
    ;; #[allow(unchecked_data)]
    (var-set reenter-staker staker)
    (var-set last-reenter-error none)
    (ok true)
  )
)

(define-public (set-withdrawal-request (wid (optional uint)))
  (begin
    ;; #[allow(unchecked_data)]
    (var-set withdrawal-id wid)
    (ok true)
  )
)

(define-read-only (get-last-reenter-error)
  (var-get last-reenter-error)
)
