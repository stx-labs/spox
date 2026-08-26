;; Trait the reward-claim-registry dispatches on to claim and settle rewards
;; for a staker via their signer-manager.
(define-trait reward-claim-signer-manager-trait (
    ;; Claim a staker's rewards for a reward cycle. Returns the net
    ;; `earned` credited to the staker and, when the payout was routed to
    ;; an L1 sBTC withdrawal, the `withdrawal-request`.
    (claim-staker-rewards
        (principal uint (optional uint))
        (response {
            earned: uint,
            withdrawal-request: (optional uint),
        } uint)
    )
    ;; Pull the signer's rewards for a reward cycle from pox-5 into the
    ;; signer-manager so per-staker claims can be paid. Must run once per
    ;; (signer, reward-cycle, scope) before claim-staker-rewards will pay;
    ;; the reward-claim-registry calls it itself when pox-5 shows that
    ;; rewards are due. The return mirrors pox-5's claim-rewards and is
    ;; unused by the registry, but the type must match for trait
    ;; conformance.
    (claim-rewards
        ((list 6 uint) uint)
        (
            response             {
            stx-rewards: {
                earned: uint,
                rewards-per-token: uint,
            },
            bond-rewards: (list 6 {
                earned: uint,
                bond-index: uint,
                rewards-per-token: uint,
            }),
            bond-totals: uint,
            total-rewards: uint,
        }
            uint
        )
    )
    ;; Settle an accepted L1 withdrawal by its sbtc-registry request-id.
    (settle-accepted-withdrawal
        (uint)
        (response bool uint)
    )
    ;; Reclaim a rejected L1 withdrawal back to the staker who earned it.
    (reclaim-failed-withdrawal
        (uint)
        (response bool uint)
    )
))

;; Cap on claim installments purchasable in one register/add-claims call.
;; Matches the longest PoX-5 STX lock (96 reward cycles = 192 distribution
;; cycles), which bounds how many distribution-aligned claims you can schedule.
(define-constant MAX_CLAIM_INSTALLMENTS u192)
;; Minimum Bitcoin blocks after indexing before get-pending-withdrawals
;; will consult sbtc-registry. Settle itself is not gated on this.
(define-constant WITHDRAWAL_MIN_BURN_AGE u7)

;; No registration for this staker and signer-manager combination
(define-constant ERR_NOT_REGISTERED (err u600))
;; The registration fee is too small to buy even one claim installment
(define-constant ERR_INSUFFICIENT_FEE (err u601))
;; The caller is not an admin to an admin only function
(define-constant ERR_NOT_ADMIN (err u602))
;; The staker has no active pox-5 position under this signer
(define-constant ERR_NO_CURRENT_POSITION (err u603))
;; The registration fee must be greater than zero
(define-constant ERR_ZERO_FEE (err u604))
;; Nothing new to claim yet: next-claim-distribution has not fully elapsed
(define-constant ERR_ALREADY_CLAIMED (err u605))
;; pox-5 calculate-rewards has not covered this registration's claim distribution
(define-constant ERR_REWARDS_NOT_CALCULATED (err u606))
;; The request-id is not a tracked pending withdrawal for this key
(define-constant ERR_UNKNOWN_PENDING_WITHDRAWAL (err u607))
;; start-reward-cycle is before the position's first-reward-cycle
(define-constant ERR_INVALID_START_REWARD_CYCLE (err u608))
;; This is returned when the contract-caller is not the sponsor for add-claims
;; calls, or not the staker or sponsor for cancel-registration calls.
(define-constant ERR_UNAUTHORIZED (err u609))
;; A registration already exists for this staker and signer-manager
(define-constant ERR_ALREADY_REGISTERED (err u610))
;; Signer-manager associated with staker does not match inputs
(define-constant ERR_SIGNER_MANAGER_MISMATCH (err u611))
;; A reentrant call into this contract was detected while a signer-manager
;; call was in flight (same pattern as pox-5's ERR_REENTRANT_CALL).
(define-constant ERR_REENTRANT_CALL (err u612))

;; A (list 100 uint) whose job is to bound the get-pending-claims,
;; get-registrations, get-pending-withdrawals, and get-withdrawals folds to
;; at most 100 iterations per call.
;; @format-ignore
(define-constant PENDING_TICKS (list
    u0 u0 u0 u0 u0 u0 u0 u0 u0 u0
    u0 u0 u0 u0 u0 u0 u0 u0 u0 u0
    u0 u0 u0 u0 u0 u0 u0 u0 u0 u0
    u0 u0 u0 u0 u0 u0 u0 u0 u0 u0
    u0 u0 u0 u0 u0 u0 u0 u0 u0 u0
    u0 u0 u0 u0 u0 u0 u0 u0 u0 u0
    u0 u0 u0 u0 u0 u0 u0 u0 u0 u0
    u0 u0 u0 u0 u0 u0 u0 u0 u0 u0
    u0 u0 u0 u0 u0 u0 u0 u0 u0 u0
    u0 u0 u0 u0 u0 u0 u0 u0 u0 u0
))

;; default to allowing deployer to register as a pool
(define-map admins
    principal
    bool
)
(map-set admins tx-sender true)

;; This is the amount of uSTX escrowed per claim installment when buying
(define-data-var fee-per-claim uint u10000)

;; Reentrancy guard: prevents cross-function re-entry through signer-manager
;; trait calls (mirrors pox-5's signer-manager-call-active).
(define-data-var signer-manager-call-active bool false)

(define-private (validate-no-reentrancy)
    (ok (asserts! (not (var-get signer-manager-call-active)) ERR_REENTRANT_CALL))
)

(define-data-var registration-ll-head (optional {
    staker: principal,
    signer-manager: principal,
}) none)
(define-data-var registration-ll-tail (optional {
    staker: principal,
    signer-manager: principal,
}) none)

(define-map registrations
    {
        staker: principal,
        signer-manager: principal,
    }
    {
        ;; The bond index of the staker's pox-5 position. None means the
        ;; position is an STX-only stake.
        bond-index: (optional uint),
        ;; The number of remaining prepaid claim installments.
        remaining-claims: uint,
        ;; When true, advance by two distribution cycles so at most one
        ;; claim runs per reward cycle. When false, up to two claims run
        ;; per reward cycle.
        one-claim-per-reward-cycle: bool,
        ;; The next distribution cycle this registration will settle.
        ;; Pending when this value is less than current-distribution-cycle and
        ;; calculate-rewards has covered through the end of this distribution.
        next-claim-distribution: uint,
        ;; The STX held by this contract for unconsumed installments.
        ;; Burned one installment at a time per processed claim. Note that
        ;; this amount is refunded to the sponsor when the registration
        ;; is cancelled.
        prepaid-ustx: uint,
        ;; The principal that created this registration and whose STX is
        ;; escrowed in prepaid-ustx. The only caller allowed to add-claims.
        ;; Cancel refunds remaining prepaid-ustx to this principal.
        sponsor: principal,
    }
)

;; Look up a registration by staker and signer-manager.
;;
;; Parameters:
;;   staker          The staker principal on the registration key.
;;   signer-manager  The signer-manager principal on the registration key.
;;
;; Returns:
;;   The registration tuple if present, otherwise none. The tuple holds
;;   bond-index, remaining-claims, one-claim-per-reward-cycle,
;;   next-claim-distribution, prepaid-ustx, and sponsor.
(define-read-only (get-registration
        (staker principal)
        (signer-manager principal)
    )
    (map-get? registrations {
        staker: staker,
        signer-manager: signer-manager,
    })
)

(define-map registration-ll
    {
        staker: principal,
        signer-manager: principal,
    }
    {
        prev: (optional {
            staker: principal,
            signer-manager: principal,
        }),
        next: (optional {
            staker: principal,
            signer-manager: principal,
        }),
    }
)

;; One row per indexed L1 withdrawal. The value is the bitcoin block-height
;; at which the withdrawal was indexed.
(define-map pending-withdrawals
    {
        staker: principal,
        signer-manager: principal,
        request-id: uint,
    }
    uint
)

(define-map pending-withdrawal-ll
    {
        staker: principal,
        signer-manager: principal,
        request-id: uint,
    }
    {
        prev: (optional {
            staker: principal,
            signer-manager: principal,
            request-id: uint,
        }),
        next: (optional {
            staker: principal,
            signer-manager: principal,
            request-id: uint,
        }),
    }
)
(define-data-var pending-withdrawal-ll-head (optional {
    staker: principal,
    signer-manager: principal,
    request-id: uint,
}) none)
(define-data-var pending-withdrawal-ll-tail (optional {
    staker: principal,
    signer-manager: principal,
    request-id: uint,
}) none)

(define-private (min-uint
        (left uint)
        (right uint)
    )
    (if (<= left right)
        left
        right
    )
)

;; Step size in distribution cycles: two when claiming at most once per reward
;; cycle, otherwise one so up to two claims per reward cycle are possible.
(define-private (claim-step (one-claim-per-reward-cycle bool))
    (if one-claim-per-reward-cycle
        u2
        u1
    )
)

;; First distribution cycle a registration covers for a start reward cycle.
;; When one-claim-per-reward-cycle is true, seed on the second half of that
;; reward cycle. Otherwise seed on the first half.
;;
;; #[allow(unchecked_data)]
(define-private (initial-next-claim-distribution
        (start-reward-cycle uint)
        (one-claim-per-reward-cycle bool)
    )
    (+ (* u2 start-reward-cycle) (- (claim-step one-claim-per-reward-cycle) u1))
)

;; Compute the next next-claim-distribution after settling claim-distribution.
;; When one-claim-per-reward-cycle is true, always advance by two. Otherwise
;; advance by one, but if the claimed reward cycle is fully past because the
;; current distribution cycle is greater than that cycle's second half, jump to
;; the first half of the following reward cycle.
;;
;; #[allow(unchecked_data)]
(define-private (next-claim-after
        (claim-distribution uint)
        (one-claim-per-reward-cycle bool)
        (current-distribution-cycle uint)
    )
    (if one-claim-per-reward-cycle
        (+ claim-distribution u2)
        (let (
                (reward-cycle (/ claim-distribution u2))
                (second-half (+ (* u2 reward-cycle) u1))
            )
            (if (> current-distribution-cycle second-half)
                (* u2 (+ reward-cycle u1))
                (+ claim-distribution u1)
            )
        )
    )
)

;; Read the current STX fee escrowed per claim installment when buying.
;;
;; Returns:
;;   The fee-per-claim data-var value in micro-STX.
(define-read-only (get-fee-per-claim)
    (var-get fee-per-claim)
)

;; --- Doubly-linked-list maintenance over registration-ll ---
;; The list lets get-pending-claims and get-registrations walk every live
;; registration without a global index. `registration-ll-head`/`-tail` bound
;; the walk; each node stores its
;; prev/next key. Append is O(1) at the tail; remove splices in O(1). Both are
;; infallible and return a boolean.

;; Append `key` at the tail (it must not already be in the list). The nested
;; match on the neighbor read avoids a runtime panic: the entry is always
;; present (it is the current tail), and the false arm is unreachable.
;;
;; #[allow(unchecked_data)]
(define-private (ll-append (key {
    staker: principal,
    signer-manager: principal,
}))
    (let ((old-tail (var-get registration-ll-tail)))
        (map-set registration-ll key {
            prev: old-tail,
            next: none,
        })
        (match old-tail
            tail-key
            (match (map-get? registration-ll tail-key)
                tail-links (map-set registration-ll tail-key (merge tail-links { next: (some key) }))
                false
            )
            ;; empty list: this node is also the head
            (var-set registration-ll-head (some key))
        )
        (var-set registration-ll-tail (some key))
    )
)

;; Splice `key` out of the list, fixing up its neighbors' links and the
;; head/tail vars. No-op if `key` isn't in the list.
;;
;; #[allow(unchecked_data)]
(define-private (ll-remove (key {
    staker: principal,
    signer-manager: principal,
}))
    (match (map-get? registration-ll key)
        links (begin
            (match (get prev links)
                prev-key (match (map-get? registration-ll prev-key)
                    prev-links (map-set registration-ll prev-key (merge prev-links { next: (get next links) }))
                    false
                )
                (var-set registration-ll-head (get next links))
            )
            (match (get next links)
                next-key (match (map-get? registration-ll next-key)
                    next-links (map-set registration-ll next-key (merge next-links { prev: (get prev links) }))
                    false
                )
                (var-set registration-ll-tail (get prev links))
            )
            (map-delete registration-ll key)
        )
        false
    )
)

;; --- Doubly-linked-list maintenance over pending-withdrawal-ll ---
;; Same shape as the registration list, one node per indexed withdrawal so
;; get-pending-withdrawals and get-withdrawals can walk them.

;; Append `key` at the tail of the pending-withdrawal list.
;;
;; #[allow(unchecked_data)]
(define-private (withdrawal-ll-append (key {
    staker: principal,
    signer-manager: principal,
    request-id: uint,
}))
    (let ((old-tail (var-get pending-withdrawal-ll-tail)))
        (map-set pending-withdrawal-ll key {
            prev: old-tail,
            next: none,
        })
        (match old-tail
            tail-key (match (map-get? pending-withdrawal-ll tail-key)
                tail-links (map-set pending-withdrawal-ll tail-key (merge tail-links { next: (some key) }))
                false
            )
            (var-set pending-withdrawal-ll-head (some key))
        )
        (var-set pending-withdrawal-ll-tail (some key))
    )
)

;; Splice `key` out of the pending-withdrawal list.
;;
;; #[allow(unchecked_data)]
(define-private (withdrawal-ll-remove (key {
    staker: principal,
    signer-manager: principal,
    request-id: uint,
}))
    (match (map-get? pending-withdrawal-ll key)
        links (begin
            (match (get prev links)
                prev-key (match (map-get? pending-withdrawal-ll prev-key)
                    prev-links (map-set pending-withdrawal-ll prev-key
                        (merge prev-links { next: (get next links) })
                    )
                    false
                )
                (var-set pending-withdrawal-ll-head (get next links))
            )
            (match (get next links)
                next-key (match (map-get? pending-withdrawal-ll next-key)
                    next-links (map-set pending-withdrawal-ll next-key
                        (merge next-links { prev: (get prev links) })
                    )
                    false
                )
                (var-set pending-withdrawal-ll-tail (get prev links))
            )
            (map-delete pending-withdrawal-ll key)
        )
        false
    )
)

;; Get the latest distribution cycle for which calculate-rewards has run.
;; Returns none when calculate-rewards has never run.
;;
;; #[allow(unchecked_data)]
(define-private (get-last-rewards-compute-distribution)
    (let ((height (contract-call? 'ST000000000000000000002AMW42H.pox-5 get-last-reward-compute-height)))
        (if (is-eq height u0)
            none
            (some (contract-call? 'ST000000000000000000002AMW42H.pox-5 burn-height-to-distribution-index
                height
            ))
        )
    )
)

;; Returns true when `last-compute-distribution` covers through the end of
;; `claim-distribution` (covered cycle >= claim-distribution).
;;
;; #[allow(unchecked_data)]
(define-private (rewards-calculated-for-claim-distribution
        (claim-distribution uint)
        (last-compute-distribution (optional uint))
    )
    (match last-compute-distribution
        covered (>= covered claim-distribution)
        false
    )
)

;; Returns true if this registration has remaining cycles left, its next claim
;; distribution is strictly before the current distribution cycle, and
;; calculate-rewards has covered that claim distribution.
(define-private (is-pending
        (registration {
            remaining-claims: uint,
            bond-index: (optional uint),
            one-claim-per-reward-cycle: bool,
            next-claim-distribution: uint,
            prepaid-ustx: uint,
            sponsor: principal,
        })
        (current-distribution-cycle uint)
        (last-compute-distribution (optional uint))
    )
    (and
        (> (get remaining-claims registration) u0)
        (< (get next-claim-distribution registration) current-distribution-cycle)
        (rewards-calculated-for-claim-distribution (get next-claim-distribution registration)
            last-compute-distribution
        )
    )
)

;; Resolve the staker's active pox-5 position. Prefers a live bond
;; membership when present, otherwise falls back to an STX-only stake. A
;; staker has at most one active bond and one active STX stake at a time.
;; Used only at registration; claims key off staker and signer-manager.
;;
;; Parameters:
;;   staker  The principal whose pox-5 position is resolved.
;;
;; Returns:
;;   none if there is no active bond and no active STX-only stake.
;;   Otherwise a tuple with signer, the signer-manager for the position,
;;   the first-reward-cycle of the stake, and the bond-index of the stake.
(define-read-only (get-position (staker principal))
    (match (contract-call? 'ST000000000000000000002AMW42H.pox-5 get-bond-membership staker)
        membership
        ;; A bond membership was found
        (some {
            signer: (get signer membership),
            first-reward-cycle: (contract-call? 'ST000000000000000000002AMW42H.pox-5 bond-period-to-reward-cycle
                (get bond-index membership)
            ),
            bond-index: (some (get bond-index membership)),
        })
        ;; No bond membership was found, look for an STX-only stake
        (match (contract-call? 'ST000000000000000000002AMW42H.pox-5 get-staker-info staker)
            info (some {
                signer: (get signer info),
                first-reward-cycle: (get first-reward-cycle info),
                bond-index: none,
            })
            none
        )
    )
)

;; Fold step for get-pending-claims. 
;;
;; From the current `node` it reads that registration, appends a row when
;; it is pending, records `last-visited`, and advances `node` to the next
;; linked-list entry. Once `node` is none it is a no-op for the remaining
;; ticks. The `last-compute-distribution` is the distribution cycle of the
;; last time that `pox-5::calculate-rewards` was run.
(define-private (pending-claims-step
        (tick_ uint)
        (acc {
            node: (optional {
                staker: principal,
                signer-manager: principal,
            }),
            last-visited: (optional {
                staker: principal,
                signer-manager: principal,
            }),
            current-distribution-cycle: uint,
            last-compute-distribution: (optional uint),
            rows: (list 100
                {
                    signer-manager: principal,
                    staker: principal,
                    bond-index: (optional uint),
                    reward-cycle: uint,
                }
            ),
        })
    )
    (match (get node acc)
        key
        (let ((next-node (match (map-get? registration-ll key)
                links (get next links)
                none
            )))
            (match (map-get? registrations key)
                registration
                (if (is-pending registration (get current-distribution-cycle acc)
                        (get last-compute-distribution acc)
                    )
                    (merge acc {
                        node: next-node,
                        last-visited: (some key),
                        rows: (default-to (get rows acc)
                            (as-max-len?
                                (append (get rows acc) {
                                    signer-manager: (get signer-manager key),
                                    staker: (get staker key),
                                    bond-index: (get bond-index registration),
                                    reward-cycle: (/ (get next-claim-distribution registration) u2),
                                })
                                u100
                            )),
                    })
                    (merge acc {
                        node: next-node,
                        last-visited: (some key),
                    })
                )
                ;; A live linked-list node with no registration should never
                ;; happen; skip it defensively rather than aborting the read.
                (merge acc {
                    node: next-node,
                    last-visited: (some key),
                })
            )
        )
        ;; Past the tail: nothing left to visit.
        acc
    )
)

;; List registrations whose next claim is pending. A registration is deemed
;; pending when:
;;
;; * The remaining-claims is greater than zero
;; * next-claim-distribution is less than the current distribution cycle
;; * pox-5::calculate-rewards has covered the distribution that will be
;;   claimed for next
;;
;; Walks the registration linked list from cursor, or from the head when
;; cursor is none, and returns up to 100 rows where the registration is
;; pending. Non-pending registrations are omitted, so a short or empty
;; `rows` list does not mean all pending registrations have been fetched.
;; Use the returned `next` cursor, where `none` means the walk hit the
;; tail. Pass `some key` as the next `cursor` to paginate all
;; registrations.
;;
;; Parameters:
;;   cursor  none to start at the head, or the `next` key from the previous
;;           page so the walk resumes at that key's successor.
;;
;; Returns:
;;   ok wrapping { rows, next }. Each row has signer-manager, staker,
;;   bond-index as none for STX-only or some index for a bond, and reward-cycle
;;   as next-claim-distribution divided by two, the pox-5 cycle to claim.
;;   `next` is none at the tail, or the last visited registration key when
;;   more nodes may remain.
(define-read-only (get-pending-claims (cursor (optional {
    staker: principal,
    signer-manager: principal,
})))
    (let (
            ;; Resume just after `cursor` (the last key the caller handled),
            ;; or start at the head on the first page.
            (start (match cursor
                cursor-key (match (map-get? registration-ll cursor-key)
                    links (get next links)
                    none
                )
                (var-get registration-ll-head)
            ))
            ;; PENDING_TICKS bounds the walk to at most 100 node visits per
            ;; call.
            (walk (fold pending-claims-step PENDING_TICKS {
                node: start,
                last-visited: none,
                current-distribution-cycle: (contract-call? 'ST000000000000000000002AMW42H.pox-5 current-distribution-cycle),
                last-compute-distribution: (get-last-rewards-compute-distribution),
                rows: (list),
            }))
            ;; If `node` is still `some` after the fold, ticks ran out with more
            ;; list ahead: resume after `last-visited`. If `node` is none, the
            ;; walk reached the tail or the list was empty.
            (next (match (get node walk)
                more-to-do (get last-visited walk)
                none
            ))
        )
        (ok {
            rows: (get rows walk),
            next: next,
        })
    )
)

;; Fold step for get-registrations. From the current `node` it reads that
;; registration, appends the full registration merged with its key, records
;; `last-visited`, and advances `node` to the next linked-list entry. Once
;; `node` is none it is a no-op for the remaining ticks.
;;
;; #[allow(unchecked_data)]
(define-private (registrations-step
        (tick_ uint)
        (acc {
            node: (optional {
                staker: principal,
                signer-manager: principal,
            }),
            last-visited: (optional {
                staker: principal,
                signer-manager: principal,
            }),
            rows: (list 100
                {
                    staker: principal,
                    signer-manager: principal,
                    bond-index: (optional uint),
                    remaining-claims: uint,
                    one-claim-per-reward-cycle: bool,
                    next-claim-distribution: uint,
                    prepaid-ustx: uint,
                    sponsor: principal,
                }
            ),
        })
    )
    (match (get node acc)
        key
        (let ((next-node (match (map-get? registration-ll key)
                links (get next links)
                none
            )))
            (match (map-get? registrations key)
                registration
                (merge acc {
                    node: next-node,
                    last-visited: (some key),
                    rows: (default-to (get rows acc)
                        (as-max-len? (append (get rows acc) (merge key registration)) u100)
                    ),
                })
                ;; A live linked-list node with no registration should never
                ;; happen; skip it defensively rather than aborting.
                (merge acc {
                    node: next-node,
                    last-visited: (some key),
                })
            )
        )
        ;; Past the tail: nothing left to visit.
        acc
    )
)

;; List every registration.

;; Walks the registration linked list from cursor, or from the head when
;; cursor is none, and returns up to 100 rows. Unlike get-pending-claims,
;; every visited registration is emitted. Use the returned `next` cursor:
;; none means the walk hit the tail; some key means pass that key as the
;; next `cursor` to resume after it.
;;
;; Parameters:
;;   cursor  none to start at the head, or the `next` key from the previous
;;           page so the walk resumes at that key's successor.
;;
;; Returns:
;;   ok wrapping { rows, next }. Each row is the registration map value with
;;   staker and signer-manager merged in. `next` is none at the tail, or the
;;   last visited registration key when more nodes may remain.
(define-read-only (get-registrations (cursor (optional {
    staker: principal,
    signer-manager: principal,
})))
    (let (
            (start (match cursor
                cursor-key (match (map-get? registration-ll cursor-key)
                    links (get next links)
                    none
                )
                (var-get registration-ll-head)
            ))
            (walk (fold registrations-step PENDING_TICKS {
                node: start,
                last-visited: none,
                rows: (list),
            }))
            (next (match (get node walk)
                more-to-do (get last-visited walk)
                none
            ))
        )
        (ok {
            rows: (get rows walk),
            next: next,
        })
    )
)

;; Set the STX fee escrowed per claim installment when buying. Admin only.
;; Changes apply only to registrations and top-ups afterward; existing
;; prepaid-ustx balances are unchanged.
;;
;; Parameters:
;;   new-fee  The new fee-per-claim in micro-STX. Must be greater than zero.
;;
;; Returns:
;;   ok true on success, or an error if the caller is not an admin or new-fee
;;   is zero.
(define-public (set-fee-per-claim (new-fee uint))
    (begin
        (try! (authorize-admin))
        (asserts! (> new-fee u0) ERR_ZERO_FEE)
        (ok (var-set fee-per-claim new-fee))
    )
)

;; --- Registration lifecycle helpers ---
;; register-for-claims / add-claims escrow STX and write the registration.
;; Advance burns one installment from escrow; cancel refunds the rest of
;; the escrowed STX to the sponsor of the registration.

;; Escrow the STX fee for the given number of claim installments unless
;; contract-caller is an admin. Returns the micro-STX amount escrowed.
(define-private (escrow-registration-fee (num-claims uint))
    (let (
            (price (var-get fee-per-claim))
            (amount (if (is-admin contract-caller)
                u0
                (* num-claims price)
            ))
        )
        (if (> amount u0)
            (try! (stx-transfer? amount contract-caller current-contract))
            true
        )
        (ok amount)
    )
)

;; Register a staker for automated reward claims. 
;;
;; See register-for-claims for documentation.
(define-private (register-for-claims-impl
        (staker principal)
        (signer-manager principal)
        (start-reward-cycle uint)
        (one-claim-per-reward-cycle bool)
        (fee uint)
    )
    (let (
            (price (var-get fee-per-claim))
            (num-claims (min-uint (/ fee price) MAX_CLAIM_INSTALLMENTS))
            (key {
                staker: staker,
                signer-manager: signer-manager,
            })
            (position (unwrap! (get-position staker) ERR_NO_CURRENT_POSITION))
        )
        (asserts! (> num-claims u0) ERR_INSUFFICIENT_FEE)
        (asserts! (is-none (map-get? registrations key)) ERR_ALREADY_REGISTERED)
        (asserts! (is-eq signer-manager (get signer position)) ERR_SIGNER_MANAGER_MISMATCH)
        (asserts! (>= start-reward-cycle (get first-reward-cycle position))
            ERR_INVALID_START_REWARD_CYCLE
        )
        (let ((escrowed (try! (escrow-registration-fee num-claims))))
            (map-set registrations key {
                bond-index: (get bond-index position),
                remaining-claims: num-claims,
                one-claim-per-reward-cycle: one-claim-per-reward-cycle,
                next-claim-distribution: (initial-next-claim-distribution start-reward-cycle one-claim-per-reward-cycle),
                prepaid-ustx: escrowed,
                sponsor: contract-caller,
            })
            (ll-append key)
            (print {
                topic: "register-for-claims",
                staker: staker,
                sponsor: contract-caller,
                signer-manager: signer-manager,
                start-reward-cycle: start-reward-cycle,
                one-claim-per-reward-cycle: one-claim-per-reward-cycle,
                num-claims: num-claims,
                escrowed: escrowed,
            })
            (ok num-claims)
        )
    )
)

;; Fold step for register-many-for-claims: match each register-for-claims-impl
;; result so a skip or failure doesn't abort the batch. Prints a skip event on
;; failure. Returns the count in the accumulator's registered field.
(define-private (count-registations
        (entry {
            staker: principal,
            start-reward-cycle: uint,
            one-claim-per-reward-cycle: bool,
            fee: uint,
        })
        (state {
            signer: principal,
            registered: uint,
        })
    )
    (match (register-for-claims-impl (get staker entry) (get signer state) (get start-reward-cycle entry)
        (get one-claim-per-reward-cycle entry) (get fee entry)
    )
        num-claims (merge state { registered: (+ (get registered state) u1) })
        err-code (begin
            (print {
                topic: "register-for-claims-skipped",
                staker: (get staker entry),
                signer-manager: (get signer state),
                start-reward-cycle: (get start-reward-cycle entry),
                one-claim-per-reward-cycle: (get one-claim-per-reward-cycle entry),
                fee: (get fee entry),
                error: err-code,
            })
            state
        )
    )
)

;; Register a staker for automated reward claims. Anyone may call this;
;; contract-caller is stored as `sponsor` and is the only principal that may
;; add-claims. Admins pay no fee. The staker must currently be staking in
;; pox-5. The active bond-index, if any, is looked up from pox-5; callers do
;; not pass it. Schedule seeds next-claim-distribution from start-reward-cycle.
;; Fee STX is escrowed in this contract and burned one installment at a time
;; when claims advance. Fails if this staker and signer-manager pair is already
;; registered; use add-claims to buy more installments.
;;
;; Parameters:
;;   staker                      The principal being registered.
;;   signer-manager              Together with staker forms the registration key.
;;                               Must be the signer pox-5 reports for the
;;                               position; every claim pulls from it.
;;   start-reward-cycle          First reward cycle on the claim schedule. Must
;;                               be greater than or equal to the position's
;;                               first-reward-cycle.
;;   one-claim-per-reward-cycle  When true, at most one claim per reward cycle:
;;                               step of two, seeded on the second half. When
;;                               false, up to two claims per reward cycle: step
;;                               of one, seeded on the first half, with catch-up
;;                               when a reward cycle is fully past.
;;   fee                         STX paid by contract-caller. Buys the minimum of
;;                               fee divided by fee-per-claim and
;;                               MAX_CLAIM_INSTALLMENTS.
;;                               Only the used portion is escrowed; any remainder
;;                               stays with the caller. Admins escrow nothing.
;;
;; Returns:
;;   ok with the number of claim installments bought on this call, or an error
;;   if fee buys no claims, a registration already exists, the position is
;;   missing or under a different signer, or start-reward-cycle is before the
;;   position's first-reward-cycle.
(define-public (register-for-claims
        (staker principal)
        (signer-manager <reward-claim-signer-manager-trait>)
        (start-reward-cycle uint)
        (one-claim-per-reward-cycle bool)
        (fee uint)
    )
    (register-for-claims-impl staker (contract-of signer-manager) start-reward-cycle
        one-claim-per-reward-cycle fee
    )
)

;; Register up to 100 stakers under the same signer-manager in one call.
;; Runs register-for-claims-impl per entry. 
;;
;; Note that a failure for any entry does not abort the batch and print
;; events are emitted for both successful and failed registrations.
;;
;; Ssee register-for-claims parameters documentation.
;;
;; Returns:
;;   ok with the number of stakers for which register-for-claims-impl
;;   returned ok.
(define-public (register-many-for-claims
        (signer-manager <reward-claim-signer-manager-trait>)
        (stakers (list 100
            {
                staker: principal,
                start-reward-cycle: uint,
                one-claim-per-reward-cycle: bool,
                fee: uint,
            }
        ))
    )
    (ok (get registered
        (fold count-registations stakers {
            signer: (contract-of signer-manager),
            registered: u0,
        })
    ))
)

;; Buy additional claim installments for an existing registration. Only the
;; stored sponsor may call this. Does not change next-claim-distribution,
;; one-claim-per-reward-cycle, bond-index, or sponsor. Fee STX is escrowed
;; and burned later when a claim is processed.
;;
;; Parameters:
;;   staker          The staker on the registration key.
;;   signer-manager  The signer-manager principal on the registration key.
;;   fee             STX paid by contract-caller. Buys the minimum of fee divided by
;;                   fee-per-claim and MAX_CLAIM_INSTALLMENTS. Only the used portion
;;                   is escrowed; any remainder stays with the caller. Admins
;;                   escrow nothing.
;;
;; Returns:
;;   ok with the number of claim installments added on this call, or an error
;;   if the caller is not the sponsor, fee buys no claims, or no registration
;;   exists for this key.
;;
;; #[allow(unchecked_data)]
(define-public (add-claims
        (staker principal)
        (signer-manager principal)
        (fee uint)
    )
    (let (
            (price (var-get fee-per-claim))
            (num-claims (min-uint (/ fee price) MAX_CLAIM_INSTALLMENTS))
            (key {
                staker: staker,
                signer-manager: signer-manager,
            })
        )
        (asserts! (> num-claims u0) ERR_INSUFFICIENT_FEE)
        ;; Fail before escrowing if this key is not registered or caller is not the sponsor.
        (let ((existing (unwrap! (map-get? registrations key) ERR_NOT_REGISTERED)))
            (asserts! (is-eq contract-caller (get sponsor existing)) ERR_UNAUTHORIZED)
            (let ((escrowed (try! (escrow-registration-fee num-claims))))
                (map-set registrations key
                    (merge existing {
                        remaining-claims: (+ (get remaining-claims existing) num-claims),
                        prepaid-ustx: (+ (get prepaid-ustx existing) escrowed),
                    })
                )
                (print {
                    topic: "add-claims",
                    staker: staker,
                    sponsor: contract-caller,
                    signer-manager: signer-manager,
                    num-claims: num-claims,
                    escrowed: escrowed,
                })
                (ok num-claims)
            )
        )
    )
)

;; Cancel a registration. The staker or the stored sponsor may call this.
;; Deletes the registration map entry and removes it from the registration
;; linked list. Refunds any remaining prepaid-ustx to the sponsor. Does not
;; touch pending L1 withdrawals for this key; those remain settleable via
;; settle-pending-withdrawal.
;;
;; Parameters:
;;   staker          The staker on the registration key.
;;   signer-manager  The signer-manager principal on the registration key.
;;
;; Returns:
;;   ok with the micro-STX refunded to the sponsor, ERR_UNAUTHORIZED
;;   if contract-caller is neither the staker nor the sponsor, or
;;   ERR_NOT_REGISTERED if no registration exists for this key.
;;
;; #[allow(unchecked_data)]
(define-public (cancel-registration
        (staker principal)
        (signer-manager principal)
    )
    (begin
        ;; ensure no reentrancy through signer-manager trait calls
        (try! (validate-no-reentrancy))
        (let (
                (key {
                    staker: staker,
                    signer-manager: signer-manager,
                })
                (registration (unwrap! (map-get? registrations key) ERR_NOT_REGISTERED))
                (refund (get prepaid-ustx registration))
                (sponsor (get sponsor registration))
            )
            (asserts! (or (is-eq contract-caller staker) (is-eq contract-caller sponsor))
                ERR_UNAUTHORIZED
            )
            (map-delete registrations key)
            (ll-remove key)
            (begin
                (if (> refund u0)
                    (try! (as-contract? ((with-stx refund))
                        (try! (stx-transfer? refund tx-sender sponsor))
                    ))
                    true
                )
                (print {
                    topic: "cancel-registration",
                    staker: staker,
                    sponsor: sponsor,
                    signer-manager: signer-manager,
                    refund: refund,
                })
                (ok refund)
            )
        )
    )
)

;; Consume one installment: burn prepaid-ustx / remaining-claims from escrow
;; (or all remaining prepaid on the last claim), then delete the registration
;; when remaining-claims would hit zero, otherwise decrement remaining-claims
;; and prepaid-ustx and advance next-claim-distribution via next-claim-after.
;;
;; #[allow(unchecked_data)]
(define-private (advance-registration
        (key {
            staker: principal,
            signer-manager: principal,
        })
        (registration {
            bond-index: (optional uint),
            remaining-claims: uint,
            one-claim-per-reward-cycle: bool,
            next-claim-distribution: uint,
            prepaid-ustx: uint,
            sponsor: principal,
        })
        (current-distribution-cycle uint)
    )
    (let (
            (remaining (get remaining-claims registration))
            (prepaid (get prepaid-ustx registration))
            (burn-amount (if (<= remaining u1)
                prepaid
                (/ prepaid remaining)
            ))
        )
        (begin
            (if (> burn-amount u0)
                (try! (as-contract? ((with-stx burn-amount))
                    (try! (stx-burn? burn-amount current-contract))
                ))
                true
            )
            (if (<= remaining u1)
                ;; last claim: drop the registration entirely
                (begin
                    (map-delete registrations key)
                    (ll-remove key)
                    (ok true)
                )
                (begin
                    (map-set registrations key
                        (merge registration {
                            remaining-claims: (- remaining u1),
                            prepaid-ustx: (- prepaid burn-amount),
                            next-claim-distribution: (next-claim-after (get next-claim-distribution registration)
                                (get one-claim-per-reward-cycle registration)
                                current-distribution-cycle
                            ),
                        })
                    )
                    (ok true)
                )
            )
        )
    )
)

;; Wrap a signer-manager `claim-rewards` call with the reentrancy guard.
;; This should be the only way claim-rewards is invoked from this contract.
;;
;; #[allow(unchecked_data)]
(define-private (signer-manager-claim-rewards
        (signer-manager <reward-claim-signer-manager-trait>)
        (bond-periods (list 6 uint))
        (reward-cycle uint)
    )
    (begin
        (asserts! (not (var-get signer-manager-call-active)) ERR_REENTRANT_CALL)
        (var-set signer-manager-call-active true)
        (let ((result (contract-call? signer-manager claim-rewards bond-periods reward-cycle)))
            (var-set signer-manager-call-active false)
            result
        )
    )
)

;; Wrap a signer-manager `claim-staker-rewards` call with the reentrancy guard.
;; This should be the only way claim-staker-rewards is invoked from this contract.
;;
;; #[allow(unchecked_data)]
(define-private (signer-manager-claim-staker-rewards
        (signer-manager <reward-claim-signer-manager-trait>)
        (staker principal)
        (reward-cycle uint)
        (bond-index (optional uint))
    )
    (begin
        (asserts! (not (var-get signer-manager-call-active)) ERR_REENTRANT_CALL)
        (var-set signer-manager-call-active true)
        (let ((result (contract-call? signer-manager claim-staker-rewards staker reward-cycle bond-index)))
            (var-set signer-manager-call-active false)
            result
        )
    )
)

;; Wrap a signer-manager `settle-accepted-withdrawal` call with the reentrancy
;; guard. This should be the only way it is invoked from this contract.
;;
;; #[allow(unchecked_data)]
(define-private (signer-manager-settle-accepted-withdrawal
        (signer-manager <reward-claim-signer-manager-trait>)
        (request-id uint)
    )
    (begin
        (asserts! (not (var-get signer-manager-call-active)) ERR_REENTRANT_CALL)
        (var-set signer-manager-call-active true)
        (let ((result (contract-call? signer-manager settle-accepted-withdrawal request-id)))
            (var-set signer-manager-call-active false)
            result
        )
    )
)

;; Wrap a signer-manager `reclaim-failed-withdrawal` call with the reentrancy
;; guard. This should be the only way it is invoked from this contract.
;;
;; #[allow(unchecked_data)]
(define-private (signer-manager-reclaim-failed-withdrawal
        (signer-manager <reward-claim-signer-manager-trait>)
        (request-id uint)
    )
    (begin
        (asserts! (not (var-get signer-manager-call-active)) ERR_REENTRANT_CALL)
        (var-set signer-manager-call-active true)
        (let ((result (contract-call? signer-manager reclaim-failed-withdrawal request-id)))
            (var-set signer-manager-call-active false)
            result
        )
    )
)

;; Used by process-reward-claim-impl after a claim-rewards pull or when none
;; was needed. Always advances one installment whether claim-staker-rewards
;; pays or errors, so an untrusted signer-manager cannot stall the registration.
;;
;; #[allow(unchecked_data)]
(define-private (claim-staker-and-advance
        (staker principal)
        (signer-manager <reward-claim-signer-manager-trait>)
        (key {
            staker: principal,
            signer-manager: principal,
        })
        (registration {
            bond-index: (optional uint),
            remaining-claims: uint,
            one-claim-per-reward-cycle: bool,
            next-claim-distribution: uint,
            prepaid-ustx: uint,
            sponsor: principal,
        })
        (reward-cycle uint)
        (claim-distribution uint)
        (bond-index (optional uint))
        (current-distribution-cycle uint)
    )
    (match (signer-manager-claim-staker-rewards signer-manager staker reward-cycle bond-index)
        claim-result
        ;; paid: advance and record any L1 withdrawal for later settlement
        (let (
                (withdrawal-request (get withdrawal-request claim-result))
                (stored (match withdrawal-request
                    id (append-pending-withdrawal key id)
                    none
                ))
            )
            (try! (advance-registration key registration current-distribution-cycle))
            (print {
                topic: "process-reward-claim",
                staker: staker,
                signer-manager: (contract-of signer-manager),
                reward-cycle: reward-cycle,
                claim-distribution: claim-distribution,
                bond-index: bond-index,
                earned: (get earned claim-result),
                claim-error: none,
                withdrawal-request: stored,
            })
            (ok stored)
        )
        err-code
        ;; not paid -- empty cycle, a zero share, or a claim failure. Advance
        ;; past it regardless so a single staker's problem never stalls the
        ;; registration or a batch; the error code rides in the event.
        (begin
            (try! (advance-registration key registration current-distribution-cycle))
            (print {
                topic: "process-reward-claim",
                staker: staker,
                signer-manager: (contract-of signer-manager),
                reward-cycle: reward-cycle,
                claim-distribution: claim-distribution,
                bond-index: bond-index,
                earned: u0,
                claim-error: (some err-code),
                withdrawal-request: none,
            })
            (ok none)
        )
    )
)

;; The one-claim primitive behind all three claim entrypoints. Looks up the
;; registration by {staker, signer-manager}. and asserts budget
;; remains and next-claim-distribution < current-distribution-cycle.
;;
;; Self-healing: if pox-5 still shows rewards owed to the signer for this cycle
;; (get-earned > u0), the signer-manager hasn't pulled them in yet, so this calls
;; `claim-rewards` itself before claiming for the staker -- the keeper never has
;; to. It is idempotent across a batch: the first staker under a given (signer,
;; cycle, scope) pulls the rewards, which drops get-earned to u0, so the rest skip
;; it. A failed pull still advances (same anti-stall rule as claim-staker-rewards):
;; the signer-manager is untrusted and must not wedge the registration. On a
;; successful pull (or when none was needed), claim-staker-and-advance runs:
;;   * ok  -- the staker was paid; record any withdrawal-request for later
;;            settlement and return it.
;;   * err -- empty cycle, a zero share, or a claim failure; advance past it
;;            anyway. The error code is surfaced in the print event.
;; The fee is burned from escrow on advance; `claim-rewards` does move sBTC
;; from pox-5 into the signer-manager.
;;
;; #[allow(unchecked_data)]
(define-private (process-reward-claim-impl
        (staker principal)
        (signer-manager <reward-claim-signer-manager-trait>)
        (current-distribution-cycle uint)
        (last-compute-distribution (optional uint))
    )
    (let (
            (signer-manager-contract (contract-of signer-manager))
            (key {
                staker: staker,
                signer-manager: signer-manager-contract,
            })
            (registration (unwrap! (map-get? registrations key) ERR_NOT_REGISTERED))
            (claim-distribution (get next-claim-distribution registration))
            (reward-cycle (/ claim-distribution u2))
            (bond-index (get bond-index registration))
        )
        (asserts! (> (get remaining-claims registration) u0) ERR_NOT_REGISTERED)
        (asserts! (< claim-distribution current-distribution-cycle) ERR_ALREADY_CLAIMED)
        (asserts!
            (rewards-calculated-for-claim-distribution claim-distribution last-compute-distribution)
            ERR_REWARDS_NOT_CALCULATED
        )
        ;; Ensure the signer-manager has pulled this cycle's rewards from pox-5.
        ;; get-earned > u0 means claim-rewards is still owed for this scope, so
        ;; pull it now (STX-stake rewards for a `none` bond, or bond `idx`).
        (if (>
                (contract-call? 'ST000000000000000000002AMW42H.pox-5 get-earned
                    signer-manager-contract reward-cycle bond-index
                )
                u0
            )
            (match (signer-manager-claim-rewards signer-manager
                (match bond-index
                    idx (list idx)
                    (list)
                )
                reward-cycle
            )
                pull-ok
                (claim-staker-and-advance staker signer-manager key registration reward-cycle
                    claim-distribution bond-index current-distribution-cycle
                )
                ;; pull failed: advance anyway so a broken or hostile signer-manager
                ;; cannot stall this registration indefinitely.
                pull-err
                (begin
                    (try! (advance-registration key registration current-distribution-cycle))
                    (print {
                        topic: "process-reward-claim",
                        staker: staker,
                        signer-manager: signer-manager-contract,
                        reward-cycle: reward-cycle,
                        claim-distribution: claim-distribution,
                        bond-index: bond-index,
                        earned: u0,
                        claim-error: (some pull-err),
                        withdrawal-request: none,
                    })
                    (ok none)
                )
            )
            (claim-staker-and-advance staker signer-manager key registration reward-cycle
                claim-distribution bond-index current-distribution-cycle
            )
        )
    )
)

;; We only want to store request-ids that are for a live sBTC withdrawal,
;; this function does that check.
;; #[allow(unchecked_data)]
(define-private (is-trackable-withdrawal (request-id uint))
    (match (contract-call? 'SM3VDXK3WZZSA84XXFKAFAF15NNZX32CTSG82JFQ4.sbtc-registry get-withdrawal-request
        request-id
    )
        request (is-none (get status request))
        false
    )
)

;; Drop the map entry and splice the node out of pending-withdrawal-ll.
;; #[allow(unchecked_data)]
(define-private (remove-pending-withdrawal (key {
    staker: principal,
    signer-manager: principal,
    request-id: uint,
}))
    (begin
        (map-delete pending-withdrawals key)
        (withdrawal-ll-remove key)
    )
)

;; Returns true when this indexed withdrawal is old enough and sBTC has
;; resolved it. Used only by get-pending-withdrawals.
;; #[allow(unchecked_data)]
(define-private (withdrawal-ready-to-list
        (request-id uint)
        (stored-height uint)
    )
    (if (< burn-block-height (+ stored-height WITHDRAWAL_MIN_BURN_AGE))
        false
        (match (contract-call? 'SM3VDXK3WZZSA84XXFKAFAF15NNZX32CTSG82JFQ4.sbtc-registry
            get-withdrawal-request request-id
        )
            request (is-some (get status request))
            false
        )
    )
)

;; Append `request-id` when it is a valid sBTC pending withdrawal. Returns
;; some id if tracked (stored or already present), none if skipped. Records
;; burn-block-height and adds an entry into the pending-withdrawal-ll
;; linked list.
;;
;; #[allow(unchecked_data)]
(define-private (append-pending-withdrawal
        (key {
            staker: principal,
            signer-manager: principal,
        })
        (request-id uint)
    )
    (let ((full-key {
            staker: (get staker key),
            signer-manager: (get signer-manager key),
            request-id: request-id,
        }))
        (if (is-trackable-withdrawal request-id)
            (if (is-some (map-get? pending-withdrawals full-key))
                (some request-id)
                (begin
                    (map-set pending-withdrawals full-key burn-block-height)
                    (withdrawal-ll-append full-key)
                    (some request-id)
                )
            )
            none
        )
    )
)

;; Claim one installment for staker under signer-manager. Permissionless. The
;; signer-manager must be passed as a trait so claim-staker-rewards and
;; claim-rewards can dispatch on it; callers typically learn the principal from
;; get-pending-claims.
;;
;; Parameters:
;;
;;   staker          The staker whose registration is claimed.
;;   signer-manager  The signer-manager trait for that registration key.
;;
;; Returns:
;;
;;   ok with some withdrawal request-id when an L1 withdrawal was stored for
;;   later settlement, ok none for a direct sBTC payout, a skipped/invalid id,
;;   or a claim path that advanced without a payout, or an error if the
;;   registration is missing, not yet pending, or pox-5 calculate-rewards has
;;   not covered this registration's claim distribution.
;;
;; #[allow(unchecked_data)]
(define-public (process-reward-claim
        (staker principal)
        (signer-manager <reward-claim-signer-manager-trait>)
    )
    (begin
        ;; ensure no reentrancy through signer-manager trait calls
        (try! (validate-no-reentrancy))
        (process-reward-claim-impl staker signer-manager
            (contract-call? 'ST000000000000000000002AMW42H.pox-5 current-distribution-cycle)
            (get-last-rewards-compute-distribution)
        )
    )
)

;; Claim installments for the given stakers, each keyed with the same
;; signer-manager. Reads pox-5's current distribution cycle and last compute
;; distribution once and threads them through. Skips without aborting the batch
;; any staker with no registration under this signer-manager, one not yet
;; pending, or one whose claim distribution has not been covered by
;; calculate-rewards. Pull and claim failures
;; still advance and count as claimed. One signer-manager per call because the
;; trait must be a single top-level argument; the keeper builds stakers from
;; that signer-manager's get-pending-claims rows.
;;
;; Parameters:
;;
;;   signer-manager  The signer-manager trait shared by every staker in the list.
;;   stakers         Up to 100 staker principals to process in order.
;;
;; Returns:
;;
;;   ok with the number of stakers for which process-reward-claim-impl returned
;;   ok, including empty-cycle and claim-error advances.
(define-public (process-reward-claims
        (signer-manager <reward-claim-signer-manager-trait>)
        (stakers (list 100 principal))
    )
    (begin
        ;; ensure no reentrancy through signer-manager trait calls
        (try! (validate-no-reentrancy))
        (ok (get claimed
            (fold count-claim stakers {
                signer-manager: signer-manager,
                current-distribution-cycle: (contract-call? 'ST000000000000000000002AMW42H.pox-5 current-distribution-cycle),
                last-compute-distribution: (get-last-rewards-compute-distribution),
                claimed: u0,
            })
        ))
    )
)

;; Fold step for process-reward-claims: match each process-reward-claim-impl result so a
;; skip or failure doesn't abort the batch. Returns the count, not the
;; accumulator, which carries the trait and can't be returned.
;;
;; #[allow(unchecked_data)]
(define-private (count-claim
        (staker principal)
        (state {
            signer-manager: <reward-claim-signer-manager-trait>,
            current-distribution-cycle: uint,
            last-compute-distribution: (optional uint),
            claimed: uint,
        })
    )
    (match (process-reward-claim-impl staker (get signer-manager state)
        (get current-distribution-cycle state) (get last-compute-distribution state)
    )
        ok-val (merge state { claimed: (+ (get claimed state) u1) })
        err-code state
    )
)

;; Fold step for get-pending-withdrawals.
;;
;; Visits one linked-list node, appends a row when the withdrawal is old
;; enough and sBTC has resolved it, records `last-visited`, and advances
;; `node`. Young or still-pending entries do not emit a row in the
;; accumulators rows field.
;;
;; #[allow(unchecked_data)]
(define-private (pending-withdrawals-step
        (tick_ uint)
        (acc {
            node: (optional {
                staker: principal,
                signer-manager: principal,
                request-id: uint,
            }),
            last-visited: (optional {
                staker: principal,
                signer-manager: principal,
                request-id: uint,
            }),
            rows: (list 100
                {
                    staker: principal,
                    signer-manager: principal,
                    request-id: uint,
                }
            ),
        })
    )
    (match (get node acc)
        key
        (let ((next-node (match (map-get? pending-withdrawal-ll key)
                links (get next links)
                none
            )))
            (match (map-get? pending-withdrawals key)
                stored-height
                (if (withdrawal-ready-to-list (get request-id key) stored-height)
                    (merge acc {
                        node: next-node,
                        last-visited: (some key),
                        rows: (default-to (get rows acc)
                            (as-max-len?
                                (append (get rows acc) {
                                    staker: (get staker key),
                                    signer-manager: (get signer-manager key),
                                    request-id: (get request-id key),
                                })
                                u100
                            )),
                    })
                    (merge acc {
                        node: next-node,
                        last-visited: (some key),
                    })
                )
                ;; A linked-list node with no pending entry should never happen;
                ;; skip it defensively rather than aborting the read.
                (merge acc {
                    node: next-node,
                    last-visited: (some key),
                })
            )
        )
        ;; Past the tail: nothing left to visit.
        acc
    )
)

;; List indexed L1 withdrawals that are ready to settle. Walks
;; pending-withdrawal-ll from cursor, or from the head when cursor is none.
;; A row is emitted only when at least WITHDRAWAL_MIN_BURN_AGE Bitcoin
;; blocks have passed since insert and sbtc-registry status indicated that
;; it has been accepted or rejected. Use the returned `next` cursor: none
;; means the walk hit the tail; some key means pass that key as the next
;; `cursor` to resume after it. Rows are included whether or not their
;; parent registration still exists.
;;
;; Parameters:
;;
;;   cursor  use none to start at the head. When some, it indicates where
;;           to resume looking for pending withdrawals. The withdrawal for
;;           this key is not included in the response.
;;
;; Returns:
;;
;;   ok wrapping { rows, next }. Each row has staker, signer-manager, and
;;   request-id. `next` is none at the tail, or the last visited key when
;;   more nodes may remain.
(define-read-only (get-pending-withdrawals (cursor (optional {
    staker: principal,
    signer-manager: principal,
    request-id: uint,
})))
    (let (
            ;; Resume just after `cursor` (the last key the caller handled), or
            ;; start at the head on the first page.
            (start (match cursor
                cursor-key (match (map-get? pending-withdrawal-ll cursor-key)
                    links (get next links)
                    none
                )
                (var-get pending-withdrawal-ll-head)
            ))
            (walk (fold pending-withdrawals-step PENDING_TICKS {
                node: start,
                last-visited: none,
                rows: (list),
            }))
            (next (match (get node walk)
                more-to-do (get last-visited walk)
                none
            ))
        )
        (ok {
            rows: (get rows walk),
            next: next,
        })
    )
)

;; Fold step for get-withdrawals. Visits one linked-list node, appends the
;; withdrawal key merged with its indexed burn height, records
;; `last-visited`, and advances `node`. Unlike pending-withdrawals-step,
;; every stored entry is emitted.
;;
;; #[allow(unchecked_data)]
(define-private (withdrawals-step
        (tick_ uint)
        (acc {
            node: (optional {
                staker: principal,
                signer-manager: principal,
                request-id: uint,
            }),
            last-visited: (optional {
                staker: principal,
                signer-manager: principal,
                request-id: uint,
            }),
            rows: (list 100
                {
                    staker: principal,
                    signer-manager: principal,
                    request-id: uint,
                    indexed-height: uint,
                }
            ),
        })
    )
    (match (get node acc)
        key
        (let ((next-node (match (map-get? pending-withdrawal-ll key)
                links (get next links)
                none
            )))
            (match (map-get? pending-withdrawals key)
                stored-height
                (merge acc {
                    node: next-node,
                    last-visited: (some key),
                    rows: (default-to (get rows acc)
                        (as-max-len?
                            (append (get rows acc) (merge key { indexed-height: stored-height }))
                            u100
                        )),
                })
                ;; A linked-list node with no pending entry should never happen;
                ;; skip it defensively rather than aborting the read.
                (merge acc {
                    node: next-node,
                    last-visited: (some key),
                })
            )
        )
        ;; Past the tail: nothing left to visit.
        acc
    )
)

;; List every indexed L1 withdrawal. Walks pending-withdrawal-ll from
;; cursor, or from the head when cursor is none, and returns up to 100
;; rows. Unlike get-pending-withdrawals, every visited entry is emitted.
;; Use the returned `next` cursor: none means the walk hit the tail; some
;; key means pass that key as the next `cursor` to resume after it.
;;
;; Parameters:
;;
;;   cursor  use none to start at the head. When some, it indicates where
;;           to resume. The withdrawal for this key is not included in the
;;           response.
;;
;; Returns:
;;
;;   ok wrapping { rows, next }. Each row has staker, signer-manager,
;;   request-id, and indexed-height (the Bitcoin block height when the
;;   withdrawal was indexed). `next` is none at the tail, or the last
;;   visited key when more nodes may remain.
(define-read-only (get-withdrawals (cursor (optional {
    staker: principal,
    signer-manager: principal,
    request-id: uint,
})))
    (let (
            (start (match cursor
                cursor-key (match (map-get? pending-withdrawal-ll cursor-key)
                    links (get next links)
                    none
                )
                (var-get pending-withdrawal-ll-head)
            ))
            (walk (fold withdrawals-step PENDING_TICKS {
                node: start,
                last-visited: none,
                rows: (list),
            }))
            (next (match (get node walk)
                more-to-do (get last-visited walk)
                none
            ))
        )
        (ok {
            rows: (get rows walk),
            next: next,
        })
    )
)

;; Shared by settle-pending-withdrawal and the batch fold below. Reads the
;; pending item's status from sbtc-registry and:
;;
;;   not indexed               ERR_UNKNOWN_PENDING_WITHDRAWAL
;;   unknown to sbtc-registry  prune the id (ok true)
;;   pending (none)            no-op (ok false)
;;   accepted (some true)      call settle-accepted-withdrawal, always prune
;;   rejected (some false)     call reclaim-failed-withdrawal, always prune
;;
;; SM errors do not influence our behavior. Returns whether the id was
;; removed (true) or is still pending (false).
;;
;; #[allow(unchecked_data)]
(define-private (settle-pending-withdrawal-impl
        (staker principal)
        (signer-manager <reward-claim-signer-manager-trait>)
        (request-id uint)
    )
    (let ((key {
            staker: staker,
            signer-manager: (contract-of signer-manager),
            request-id: request-id,
        }))
        (asserts! (is-some (map-get? pending-withdrawals key)) ERR_UNKNOWN_PENDING_WITHDRAWAL)
        (match (contract-call? 'SM3VDXK3WZZSA84XXFKAFAF15NNZX32CTSG82JFQ4.sbtc-registry
            get-withdrawal-request request-id
        )
            request (match (get status request)
                accepted (begin
                    (if accepted
                        (is-ok (signer-manager-settle-accepted-withdrawal signer-manager request-id))
                        (is-ok (signer-manager-reclaim-failed-withdrawal signer-manager request-id))
                    )
                    (remove-pending-withdrawal key)
                    (print {
                        topic: "settle-pending-withdrawal",
                        staker: staker,
                        signer-manager: (contract-of signer-manager),
                        request-id: request-id,
                        accepted: accepted,
                    })
                    (ok true)
                )
                (ok false)
            )
            (begin
                (remove-pending-withdrawal key)
                (print {
                    topic: "prune-pending-withdrawal",
                    staker: staker,
                    signer-manager: (contract-of signer-manager),
                    request-id: request-id,
                })
                (ok true)
            )
        )
    )
)

;; Resolve one pending withdrawal. Reads its status from sbtc-registry. When
;; the id is unknown to sbtc-registry it is pruned. When status is none the
;; call is a no-op. When accepted or rejected, calls the signer-manager then
;; always removes the id (even if the SM errors).
;;
;; Parameters:
;;   staker          The staker on the pending-withdrawal key.
;;   signer-manager  The signer-manager trait for that key.
;;   request-id      The sbtc-registry withdrawal request-id to settle.
;;
;; Returns:
;;   ok true if the id was removed (settled, pruned, or SM error), ok false if
;;   it is still pending in sbtc-registry, or an error if the request-id is not
;;   tracked for this staker and signer-manager.
;;
;; #[allow(unchecked_data)]
(define-public (settle-pending-withdrawal
        (staker principal)
        (signer-manager <reward-claim-signer-manager-trait>)
        (request-id uint)
    )
    (begin
        ;; ensure no reentrancy through signer-manager trait calls
        (try! (validate-no-reentrancy))
        (settle-pending-withdrawal-impl staker signer-manager request-id)
    )
)

;; Batch settle-pending-withdrawal for one signer-manager. The trait must be a
;; single top-level argument, matching process-reward-claims. Skips without
;; aborting the batch any item that is not found or still pending.
;;
;; Parameters:
;;   signer-manager  The signer-manager trait shared by every item.
;;   items           Up to 100 tuples of staker and request-id.
;;
;; Returns:
;;   ok with the number of items that were removed from the pending list.
(define-public (settle-pending-withdrawals
        (signer-manager <reward-claim-signer-manager-trait>)
        (items (list 100 {
            staker: principal,
            request-id: uint,
        }))
    )
    (begin
        ;; ensure no reentrancy through signer-manager trait calls
        (try! (validate-no-reentrancy))
        (ok (get resolved
            (fold count-settlement items {
                signer-manager: signer-manager,
                resolved: u0,
            })
        ))
    )
)

;; Fold step for settle-pending-withdrawals: match each
;; settle-pending-withdrawal-impl result so a skip or failure doesn't abort
;; the batch. Returns the count, not the accumulator, which carries the
;; trait and can't be returned.
;; #[allow(unchecked_data)]
(define-private (count-settlement
        (item {
            staker: principal,
            request-id: uint,
        })
        (state {
            signer-manager: <reward-claim-signer-manager-trait>,
            resolved: uint,
        })
    )
    (match (settle-pending-withdrawal-impl (get staker item) (get signer-manager state)
        (get request-id item)
    )
        did-resolve (merge state { resolved: (+ (get resolved state) (if did-resolve
            u1
            u0
        )) }
        )
        err-code state
    )
)

;;; Admin functions

;; Grant or revoke an admin. Caller must already be an admin.
;;
;; Parameters:
;;   admin    The principal whose admin status is updated.
;;   enabled  true to grant admin, false to revoke.
;;
;; Returns:
;;   ok with the admin principal on success, or an error if the caller is not
;;   an admin.
;;
;; #[allow(unchecked_data)]
(define-public (update-admin
        (admin principal)
        (enabled bool)
    )
    (begin
        (try! (authorize-admin))
        (print {
            topic: "update-admin",
            admin: admin,
            enabled: enabled,
        })
        (map-set admins admin enabled)
        (ok admin)
    )
)

;; Check whether a principal is an admin of this contract.
;;
;; Parameters:
;;   caller  The principal to check.
;;
;; Returns:
;;   true if caller is marked admin, otherwise false.
(define-read-only (is-admin (caller principal))
    (default-to false (map-get? admins caller))
)

(define-private (authorize-admin)
    (ok (asserts! (and (is-eq contract-caller tx-sender) (is-admin tx-sender)) ERR_NOT_ADMIN))
)
