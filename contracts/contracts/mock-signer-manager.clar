;; title: mock-signer-manager
;; summary: Test double for reward-claim-registry. Implements both the
;; reward-claim trait and pox-5's signer-manager-trait so tests can stake under
;; this principal, leave get-earned > 0, and inject failures from claim-rewards
;; and/or claim-staker-rewards independently.

(impl-trait .reward-claim-traits.reward-claim-signer-manager-trait)
(impl-trait 'ST000000000000000000002AMW42H.pox-5.signer-manager-trait)
(use-trait signer-manager-trait 'ST000000000000000000002AMW42H.pox-5.signer-manager-trait)

(define-data-var claim-rewards-should-err bool false)
(define-data-var claim-rewards-err-code uint u1001)

(define-data-var claim-staker-should-err bool false)
(define-data-var claim-staker-err-code uint u1001)
(define-data-var earned-amount uint u1000)
(define-data-var withdrawal-id (optional uint) none)

(define-data-var settle-should-err bool false)
(define-data-var settle-err-code uint u1001)

;; pox-5 callback: always accept the stake so fixtures can register positions
;; under this mock without real signer-manager bookkeeping.
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

;; Register this contract as a pox-5 signer (mirrors signer-manager.register-self).
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

;; #[allow(unnecessary_public)]
(define-public (claim-staker-rewards
    ;; #[allow(unused_binding)]
    (staker principal)
    ;; #[allow(unused_binding)]
    (reward-cycle uint)
    ;; #[allow(unused_binding)]
    (bond-index (optional uint))
  )
  (if (var-get claim-staker-should-err)
    (err (var-get claim-staker-err-code))
    (ok {
      earned: (var-get earned-amount),
      withdrawal-request: (var-get withdrawal-id),
    })
  )
)

;; Stub return shape matches the trait; registry ignores the payload.
;; #[allow(unnecessary_public)]
(define-public (claim-rewards
    ;; #[allow(unused_binding)]
    (bond-periods (list 6 uint))
    ;; #[allow(unused_binding)]
    (reward-cycle uint)
  )
  (if (var-get claim-rewards-should-err)
    (err (var-get claim-rewards-err-code))
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
  (if (var-get settle-should-err)
    (err (var-get settle-err-code))
    (ok true)
  )
)

;; #[allow(unnecessary_public)]
(define-public (reclaim-failed-withdrawal
  ;; #[allow(unused_binding)]
  (request-id uint)
)
  (if (var-get settle-should-err)
    (err (var-get settle-err-code))
    (ok true)
  )
)

;; --- test setters ---

;; Configure claim-staker-rewards: return (err code) when should-error, else
;; (ok { earned, withdrawal-request: wid }).
(define-public (set-claim-staker-result
    (should-error bool)
    (code uint)
    (earned uint)
    (wid (optional uint))
  )
  (begin
    (var-set claim-staker-should-err should-error)
    ;; #[allow(unchecked_data)]
    (var-set claim-staker-err-code code)
    ;; #[allow(unchecked_data)]
    (var-set earned-amount earned)
    ;; #[allow(unchecked_data)]
    (var-set withdrawal-id wid)
    (ok true)
  )
)

;; Configure claim-rewards: return (err code) when should-error, else ok stub.
(define-public (set-claim-rewards-result
    (should-error bool)
    (code uint)
  )
  (begin
    (var-set claim-rewards-should-err should-error)
    ;; #[allow(unchecked_data)]
    (var-set claim-rewards-err-code code)
    (ok true)
  )
)

;; Configure settle-accepted-withdrawal / reclaim-failed-withdrawal:
;; return (err code) when should-error, else (ok true).
(define-public (set-settle-result
    (should-error bool)
    (code uint)
  )
  (begin
    (var-set settle-should-err should-error)
    ;; #[allow(unchecked_data)]
    (var-set settle-err-code code)
    (ok true)
  )
)

;; Back-compat alias for older tests that only drove claim-staker-rewards.
(define-public (set-claim-result
    (should-error bool)
    (code uint)
    (earned uint)
    (wid (optional uint))
  )
  (set-claim-staker-result should-error code earned wid)
)
