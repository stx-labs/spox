(use-trait reward-claim-signer-manager-trait .reward-claim-traits.reward-claim-signer-manager-trait)

;; The longest STX lock in PoX-5 is 96 reward cycles, which equals 192 distribution cycles
(define-constant MAX_DISTRIBUTION_CYCLES u192)

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
;; This is thrown when there are more than 192 pending withdrawals for a
;; registrant. This should not be reachable during PoX-5, which should not
;; be around for more than 96 reward cycles.
(define-constant ERR_TOO_MANY_PENDING (err u606))
;; The request-id is not a tracked pending withdrawal for this key
(define-constant ERR_UNKNOWN_PENDING_WITHDRAWAL (err u607))
;; start-reward-cycle is before the position's first-reward-cycle
(define-constant ERR_INVALID_START_REWARD_CYCLE (err u608))
;; This is returned when the tx-sender is not allowed to register, add claims,
;; or cancel for this staker.
(define-constant ERR_UNAUTHORIZED (err u609))
;; A registration already exists for this staker and signer-manager
(define-constant ERR_ALREADY_REGISTERED (err u610))
;; Signer-manager associated with staker does not match inputs
(define-constant ERR_SIGNER_MANAGER_MISMATCH (err u611))

;; A (list 100 uint) whose only job is to bound the get-pending-claims /
;; get-pending-settlements folds to at most 100 node visits per call. The
;; element values are never read (the fold step ignores `tick`).
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
(define-data-var fee-per-cycle uint u100000)

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
        bond-index: (optional uint),
        remaining-cycles: uint,
        ;; When true, advance by two distribution cycles so at most one claim
        ;; runs per reward cycle, seeded on the second half. When false,
        ;; advance by one so up to two claims run per reward cycle, seeded on
        ;; the first half, jumping to the next reward cycle's first half when
        ;; catching up past a finished cycle.
        one-claim-per-reward-cycle: bool,
        ;; The next distribution cycle this registration will settle.
        ;; Pending when this value is less than current-distribution-cycle.
        next-claim-distribution: uint,
        ;; STX held by this contract for unconsumed installments. Burned one
        ;; installment at a time on advance; refunded to the staker on cancel.
        prepaid-ustx: uint,
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
;;   bond-index, remaining-cycles, one-claim-per-reward-cycle,
;;   next-claim-distribution, and prepaid-ustx.
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

(define-map pending-withdrawals
    {
        staker: principal,
        signer-manager: principal,
    }
    (list 192 uint)
)

(define-map pending-withdrawal-ll
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
(define-data-var pending-withdrawal-ll-head (optional {
    staker: principal,
    signer-manager: principal,
}) none)
(define-data-var pending-withdrawal-ll-tail (optional {
    staker: principal,
    signer-manager: principal,
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
;;   The fee-per-cycle data-var value in micro-STX.
(define-read-only (get-fee-per-cycle)
    (var-get fee-per-cycle)
)

;; --- Doubly-linked-list maintenance over registration-ll ---
;; The list lets get-pending-claims walk every live registration without a global
;; index. `registration-ll-head`/`-tail` bound the walk; each node stores its
;; prev/next key. Append is O(1) at the tail; remove splices in O(1). Both are
;; infallible and return bool.

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
;; Same shape as the registration list, but tracking only keys with at least
;; one outstanding withdrawal so get-pending-settlements can walk them directly.

;; Append `key` at the tail of the pending-withdrawal list.
;;
;; #[allow(unchecked_data)]
(define-private (pending-ll-append (key {
    staker: principal,
    signer-manager: principal,
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
(define-private (pending-ll-remove (key {
    staker: principal,
    signer-manager: principal,
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

;; Returns true if this registration has remaining cycles left and its next
;; claim distribution is strictly before the current distribution cycle.
(define-private (is-pending
        (registration {
            remaining-cycles: uint,
            bond-index: (optional uint),
            one-claim-per-reward-cycle: bool,
            next-claim-distribution: uint,
            prepaid-ustx: uint,
        })
        (current-distribution-cycle uint)
    )
    (and
        (> (get remaining-cycles registration) u0)
        (< (get next-claim-distribution registration) current-distribution-cycle)
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

;; Fold step for get-pending-claims. From the current `node` it reads that
;; registration, appends a row when it is pending, records `last-visited`, and
;; advances `node` to the next linked-list entry. Once `node` is none (walked
;; past the tail) it is a no-op for the remaining ticks.
;; `current-distribution-cycle` rides in the accumulator so the pending check
;; never re-reads it. `tick` is unused: the tick list only bounds iterations.
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
                (if (is-pending registration (get current-distribution-cycle acc))
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

;; List registrations whose next claim is pending. Walks the registration
;; linked list from cursor, or from the head when cursor is none, and
;; returns up to 100 rows where remaining-cycles is greater than zero and
;; next-claim-distribution is less than the current distribution cycle.
;; Non-pending registrations still consume walk ticks without appending a
;; row, so a short or empty `rows` list does not mean the tail was reached.
;; Use the returned `next` cursor: none means the walk hit the tail; some key
;; means pass that key as the next `cursor` to resume after it.
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
            ;; PENDING_TICKS is the (list 100 uint) that bounds the walk to at
            ;; most 100 node visits per call. `current-distribution-cycle` is
            ;; read once here and threaded through the fold.
            (walk (fold pending-claims-step PENDING_TICKS {
                node: start,
                last-visited: none,
                current-distribution-cycle: (contract-call? 'ST000000000000000000002AMW42H.pox-5 current-distribution-cycle),
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

;; Set the STX fee escrowed per claim installment when buying. Admin only.
;; Changes apply only to registrations and top-ups afterward; existing
;; prepaid-ustx balances are unchanged.
;;
;; Parameters:
;;   new-fee  The new fee-per-cycle in micro-STX. Must be greater than zero.
;;
;; Returns:
;;   ok true on success, or an error if the caller is not an admin or new-fee
;;   is zero.
(define-public (set-fee-per-cycle (new-fee uint))
    (begin
        (try! (authorize-admin))
        (asserts! (> new-fee u0) ERR_ZERO_FEE)
        (ok (var-set fee-per-cycle new-fee))
    )
)

;; --- Registration lifecycle helpers ---
;; register-for-claims / add-claims escrow STX and write the registration.
;; Advance burns one installment from escrow; cancel refunds the rest.

;; Staker may act on their own registration; admins may register or top up
;; anyone. Cancel stays staker-only (see cancel-registration).
(define-private (authorize-staker-or-admin (staker principal))
    (ok (asserts! (or (is-eq tx-sender staker) (is-admin tx-sender)) ERR_UNAUTHORIZED))
)

;; Escrow the STX fee for num-cycles installments unless tx-sender is an admin.
;; Transfers into this contract. Returns the micro-STX amount escrowed.
(define-private (escrow-registration-fee (num-cycles uint))
    (let (
            (price (var-get fee-per-cycle))
            (amount (if (is-admin tx-sender)
                u0
                (* num-cycles price)
            ))
        )
        (if (> amount u0)
            (try! (stx-transfer? amount tx-sender current-contract))
            true
        )
        (ok amount)
    )
)

;; Register a staker for automated reward claims. Only the staker or an admin
;; may call this. Admins pay no fee. The staker must currently be staking in
;; pox-5. The active bond-index, if any, is looked up from pox-5; callers do
;; not pass it. Schedule seeds next-claim-distribution from start-reward-cycle.
;; Fee STX is escrowed in this contract and burned one installment at a time
;; when claims advance. Fails if this staker and signer-manager pair is already
;; registered; use add-claims to buy more installments.
;;
;; Parameters:
;;   staker                      The principal being registered. Must equal
;;                               tx-sender unless the caller is an admin.
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
;;   fee                         STX paid by tx-sender. Buys the minimum of fee
;;                               divided by fee-per-cycle and 192 installments.
;;                               Only the used portion is escrowed; any remainder
;;                               stays with the caller. Admins escrow nothing.
;;
;; Returns:
;;   ok with the number of claim installments bought on this call, or an error
;;   if the caller is unauthorized, fee buys no claims, a registration already
;;   exists, the position is missing or under a different signer, or
;;   start-reward-cycle is before the position's first-reward-cycle.
(define-public (register-for-claims
        (staker principal)
        (signer-manager <reward-claim-signer-manager-trait>)
        (start-reward-cycle uint)
        (one-claim-per-reward-cycle bool)
        (fee uint)
    )
    (let (
            (price (var-get fee-per-cycle))
            (num-cycles (min-uint (/ fee price) MAX_DISTRIBUTION_CYCLES))
            (signer (contract-of signer-manager))
            (key {
                staker: staker,
                signer-manager: signer,
            })
            (position (unwrap! (get-position staker) ERR_NO_CURRENT_POSITION))
        )
        (try! (authorize-staker-or-admin staker))
        ;; Validate before escrowing.
        (asserts! (> num-cycles u0) ERR_INSUFFICIENT_FEE)
        (asserts! (is-none (map-get? registrations key)) ERR_ALREADY_REGISTERED)
        (asserts! (is-eq signer (get signer position)) ERR_SIGNER_MANAGER_MISMATCH)
        (asserts! (>= start-reward-cycle (get first-reward-cycle position))
            ERR_INVALID_START_REWARD_CYCLE
        )
        (let ((escrowed (try! (escrow-registration-fee num-cycles))))
            (map-set registrations key {
                bond-index: (get bond-index position),
                remaining-cycles: num-cycles,
                one-claim-per-reward-cycle: one-claim-per-reward-cycle,
                next-claim-distribution: (initial-next-claim-distribution start-reward-cycle one-claim-per-reward-cycle),
                prepaid-ustx: escrowed,
            })
            (ll-append key)
            (print {
                topic: "register-for-claims",
                staker: staker,
                registrant: tx-sender,
                signer-manager: signer,
                start-reward-cycle: start-reward-cycle,
                one-claim-per-reward-cycle: one-claim-per-reward-cycle,
                num-cycles: num-cycles,
                escrowed: escrowed,
            })
            (ok num-cycles)
        )
    )
)

;; Buy additional claim installments for an existing registration. Only the
;; staker or an admin may call this. Does not change next-claim-distribution,
;; one-claim-per-reward-cycle, or bond-index. Fee STX is escrowed and burned
;; later on advance.
;;
;; Parameters:
;;   staker          The staker on the registration key. Must equal tx-sender
;;                   unless the caller is an admin.
;;   signer-manager  The signer-manager principal on the registration key.
;;   fee             STX paid by tx-sender. Buys the minimum of fee divided by
;;                   fee-per-cycle and 192 installments. Only the used portion
;;                   is escrowed; any remainder stays with the caller. Admins
;;                   escrow nothing.
;;
;; Returns:
;;   ok with the number of claim installments added on this call, or an error
;;   if the caller is unauthorized, fee buys no claims, or no registration
;;   exists for this key.
;;
;; #[allow(unchecked_data)]
(define-public (add-claims
        (staker principal)
        (signer-manager principal)
        (fee uint)
    )
    (let (
            (price (var-get fee-per-cycle))
            (num-cycles (min-uint (/ fee price) MAX_DISTRIBUTION_CYCLES))
            (key {
                staker: staker,
                signer-manager: signer-manager,
            })
        )
        (asserts! (> num-cycles u0) ERR_INSUFFICIENT_FEE)
        (try! (authorize-staker-or-admin staker))
        ;; Fail before escrowing if this key is not registered.
        (let (
                (existing (unwrap! (map-get? registrations key) ERR_NOT_REGISTERED))
                (escrowed (try! (escrow-registration-fee num-cycles)))
            )
            (map-set registrations key
                (merge existing {
                    remaining-cycles: (+ (get remaining-cycles existing) num-cycles),
                    prepaid-ustx: (+ (get prepaid-ustx existing) escrowed),
                })
            )
            (print {
                topic: "add-claims",
                staker: staker,
                payer: tx-sender,
                signer-manager: signer-manager,
                num-cycles: num-cycles,
                escrowed: escrowed,
            })
            (ok num-cycles)
        )
    )
)

;; Cancel a registration. Only the staker may call this - not an admin, even
;; if the admin created the registration. Deletes the registration map entry
;; and removes it from the registration linked list. Refunds any remaining
;; prepaid-ustx to the staker. Does not touch pending L1 withdrawals for this
;; key; those remain settleable via settle-pending-withdrawal.
;;
;; Parameters:
;;   staker          The staker on the registration key. Must equal tx-sender.
;;   signer-manager  The signer-manager principal on the registration key.
;;
;; Returns:
;;   ok with the micro-STX refunded to the staker, ERR_UNAUTHORIZED if
;;   tx-sender is not the staker, or ERR_NOT_REGISTERED if no registration
;;   exists for this key.
;;
;; #[allow(unchecked_data)]
(define-public (cancel-registration
        (staker principal)
        (signer-manager principal)
    )
    (let (
            (key {
                staker: staker,
                signer-manager: signer-manager,
            })
            (registration (unwrap! (map-get? registrations key) ERR_NOT_REGISTERED))
            (refund (get prepaid-ustx registration))
        )
        (asserts! (is-eq tx-sender staker) ERR_UNAUTHORIZED)
        (map-delete registrations key)
        (ll-remove key)
        (begin
            (if (> refund u0)
                (try! (as-contract? ((with-stx refund)) (try! (stx-transfer? refund tx-sender staker))))
                true
            )
            (print {
                topic: "cancel-registration",
                staker: staker,
                signer-manager: signer-manager,
                refund: refund,
            })
            (ok refund)
        )
    )
)

;; Consume one installment: burn prepaid-ustx / remaining-cycles from escrow
;; (or all remaining prepaid on the last claim), then delete the registration
;; when remaining-cycles would hit zero, otherwise decrement remaining-cycles
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
            remaining-cycles: uint,
            one-claim-per-reward-cycle: bool,
            next-claim-distribution: uint,
            prepaid-ustx: uint,
        })
        (current-distribution-cycle uint)
    )
    (let (
            (remaining (get remaining-cycles registration))
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
                            remaining-cycles: (- remaining u1),
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
            remaining-cycles: uint,
            one-claim-per-reward-cycle: bool,
            next-claim-distribution: uint,
            prepaid-ustx: uint,
        })
        (reward-cycle uint)
        (claim-distribution uint)
        (bond-index (optional uint))
        (current-distribution-cycle uint)
    )
    (match (contract-call? signer-manager claim-staker-rewards staker reward-cycle bond-index)
        claim-result
        ;; paid: advance and record any L1 withdrawal for later settlement
        (let ((withdrawal-request (get withdrawal-request claim-result)))
            (try! (advance-registration key registration current-distribution-cycle))
            (match withdrawal-request
                id (try! (append-pending-withdrawal key id))
                true
            )
            (print {
                topic: "process-reward-claim",
                staker: staker,
                signer-manager: (contract-of signer-manager),
                reward-cycle: reward-cycle,
                claim-distribution: claim-distribution,
                bond-index: bond-index,
                earned: (get earned claim-result),
                claim-error: none,
                withdrawal-request: withdrawal-request,
            })
            (ok withdrawal-request)
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
        (asserts! (> (get remaining-cycles registration) u0) ERR_NOT_REGISTERED)
        (asserts! (< claim-distribution current-distribution-cycle) ERR_ALREADY_CLAIMED)
        ;; Ensure the signer-manager has pulled this cycle's rewards from pox-5.
        ;; get-earned > u0 means claim-rewards is still owed for this scope, so
        ;; pull it now (STX-stake rewards for a `none` bond, or bond `idx`).
        (if (>
                (contract-call? 'ST000000000000000000002AMW42H.pox-5 get-earned
                    signer-manager-contract reward-cycle bond-index
                )
                u0
            )
            (match (contract-call? signer-manager claim-rewards
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

;; Bookkeeping only. Appends `request-id` to key's pending-withdrawals entry
;; (append + as-max-len? back to 192), erroring ERR_TOO_MANY_PENDING if full.
;; Splices key into pending-withdrawal-ll if this is its first pending item.
;;
;; #[allow(unchecked_data)]
(define-private (append-pending-withdrawal
        (key {
            staker: principal,
            signer-manager: principal,
        })
        (request-id uint)
    )
    (let (
            (current (default-to (list) (map-get? pending-withdrawals key)))
            (was-empty (is-eq current (list)))
            (updated (unwrap! (as-max-len? (append current request-id) u192) ERR_TOO_MANY_PENDING))
        )
        (map-set pending-withdrawals key updated)
        (if was-empty
            (pending-ll-append key)
            true
        )
        (ok true)
    )
)

;; Claim one installment for staker under signer-manager. Permissionless. The
;; signer-manager must be passed as a trait so claim-staker-rewards and
;; claim-rewards can dispatch on it; callers typically learn the principal from
;; get-pending-claims. Reads pox-5's current distribution cycle and delegates to
;; process-reward-claim-impl.
;;
;; Parameters:
;;
;;   staker          The staker whose registration is claimed.
;;   signer-manager  The signer-manager trait for that registration key.
;;
;; Returns:
;;
;;   ok with some withdrawal request-id when an L1 withdrawal was initiated,
;;   ok none for a direct sBTC payout or when the claim path advanced without
;;   a payout, or an error if the registration is missing or not yet pending.
;;
;; #[allow(unchecked_data)]
(define-public (process-reward-claim
        (staker principal)
        (signer-manager <reward-claim-signer-manager-trait>)
    )
    (process-reward-claim-impl staker signer-manager
        (contract-call? 'ST000000000000000000002AMW42H.pox-5 current-distribution-cycle)
    )
)

;; Claim installments for the given stakers, each keyed with the same
;; signer-manager. Reads pox-5's current distribution cycle once and threads it
;; through. Skips without aborting the batch any staker with no registration
;; under this signer-manager or one not yet pending. Pull and claim failures
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
;;   ok with the number of stakers for which process-reward-claim-impl returned
;;   ok, including empty-cycle and claim-error advances.
(define-public (process-reward-claims
        (signer-manager <reward-claim-signer-manager-trait>)
        (stakers (list 100 principal))
    )
    (ok (get claimed
        (fold count-claim stakers {
            signer-manager: signer-manager,
            current-distribution-cycle: (contract-call? 'ST000000000000000000002AMW42H.pox-5 current-distribution-cycle),
            claimed: u0,
        })
    ))
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
            claimed: uint,
        })
    )
    (match (process-reward-claim-impl staker (get signer-manager state)
        (get current-distribution-cycle state)
    )
        ok-val (merge state { claimed: (+ (get claimed state) u1) })
        err-code state
    )
)

;; Fold step for get-pending-settlements. Reads the current node's pending
;; request-ids, appends one row carrying the whole list, and advances `node`
;; to the next entry. Every node in pending-withdrawal-ll has a nonempty
;; pending-withdrawals entry (a node is spliced out when its list empties), so
;; no filtering is needed and one row is emitted per node. Once `node` is none
;; it is a no-op. `tick` is unused; it only bounds the iteration count.
;;
;; #[allow(unchecked_data)]
(define-private (pending-settlements-step
        (tick_ uint)
        (acc {
            node: (optional {
                staker: principal,
                signer-manager: principal,
            }),
            rows: (list 100
                {
                    staker: principal,
                    signer-manager: principal,
                    request-ids: (list 192 uint),
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
                request-ids
                (merge acc {
                    node: next-node,
                    rows: (default-to (get rows acc)
                        (as-max-len?
                            (append (get rows acc) {
                                staker: (get staker key),
                                signer-manager: (get signer-manager key),
                                request-ids: request-ids,
                            })
                            u100
                        )),
                })
                ;; A linked-list node with no pending entry should never happen;
                ;; skip it defensively rather than aborting the read.
                (merge acc { node: next-node })
            )
        )
        ;; Past the tail: nothing left to visit.
        acc
    )
)

;; List keys with outstanding L1 withdrawal request-ids. Walks
;; pending-withdrawal-ll from cursor, or from the head when cursor is none, and
;; returns up to 100 rows. Every node has a nonempty pending-withdrawals entry,
;; so each visited node yields exactly one row and a short page means the tail
;; was reached. Rows are included whether or not their parent registration still
;; exists. Does not check sbtc-registry status; the caller should check each
;; request-id before paying gas to settle.
;;
;; Parameters:
;;   cursor  none to start at the head, or the last key from the previous page
;;           so the walk resumes at that key's successor.
;;
;; Returns:
;;   ok wrapping a list of rows. Each row has staker, signer-manager, and
;;   request-ids, every sbtc-registry request-id awaiting settlement for that
;;   key, up to 192.
(define-read-only (get-pending-settlements (cursor (optional {
    staker: principal,
    signer-manager: principal,
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
        )
        ;; PENDING_TICKS is the elided (list 100 uint) bounding the walk to at most
        ;; 100 node visits per call.
        (ok (get rows
            (fold pending-settlements-step PENDING_TICKS {
                node: start,
                rows: (list),
            })
        ))
    )
)

;; Fold step for settle-pending-withdrawal-impl: drops `target` from the list
;; being rebuilt, recording whether it was found.
(define-private (pending-withdrawal-fold-step
        (request-id uint)
        (acc {
            target: uint,
            found: bool,
            kept: (list 192 uint),
        })
    )
    (if (is-eq request-id (get target acc))
        (merge acc { found: true })
        (merge acc { kept: (default-to (get kept acc) (as-max-len? (append (get kept acc) request-id) u192)) })
    )
)

;; Shared by settle-pending-withdrawal and the batch fold below. Reads the
;; pending item's status from sbtc-registry and:
;;   pending (status none)     no-op. Calling early just costs the caller's gas.
;;   accepted (some true)      calls signer-manager::settle-accepted-withdrawal.
;;   rejected (some false)     calls signer-manager::reclaim-failed-withdrawal.
;; Either resolved case removes the request-id from pending-withdrawals (deleting
;; the entry and splicing out of pending-withdrawal-ll if that empties the list).
;; No STX moves. Returns whether it resolved (true) or was still pending (false).
;;
;; #[allow(unchecked_data)]
(define-private (settle-pending-withdrawal-impl
        (staker principal)
        (signer-manager <reward-claim-signer-manager-trait>)
        (request-id uint)
    )
    (let (
            (key {
                staker: staker,
                signer-manager: (contract-of signer-manager),
            })
            (current (unwrap! (map-get? pending-withdrawals key) ERR_UNKNOWN_PENDING_WITHDRAWAL))
            (fold-result (fold pending-withdrawal-fold-step current {
                target: request-id,
                found: false,
                kept: (list),
            }))
            (request (unwrap!
                (contract-call? 'SM3VDXK3WZZSA84XXFKAFAF15NNZX32CTSG82JFQ4.sbtc-registry
                    get-withdrawal-request request-id
                )
                ERR_UNKNOWN_PENDING_WITHDRAWAL
            ))
        )
        (asserts! (get found fold-result) ERR_UNKNOWN_PENDING_WITHDRAWAL)
        (match (get status request)
            accepted (begin
                (if accepted
                    (try! (contract-call? signer-manager settle-accepted-withdrawal request-id))
                    (try! (contract-call? signer-manager reclaim-failed-withdrawal request-id))
                )
                (if (is-eq (get kept fold-result) (list))
                    (begin
                        (map-delete pending-withdrawals key)
                        (pending-ll-remove key)
                    )
                    (map-set pending-withdrawals key (get kept fold-result))
                )
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
    )
)

;; Resolve one pending withdrawal. Reads its status from sbtc-registry. When
;; status is none the call is a no-op. When accepted, calls the signer-manager's
;; settle-accepted-withdrawal. When rejected, calls reclaim-failed-withdrawal.
;; Either resolved case removes the request-id from pending-withdrawals and
;; splices the key out of pending-withdrawal-ll if that list becomes empty.
;; Permissionless; the caller pays gas and receives nothing.
;;
;; Parameters:
;;   staker          The staker on the pending-withdrawal key.
;;   signer-manager  The signer-manager trait for that key.
;;   request-id      The sbtc-registry withdrawal request-id to settle.
;;
;; Returns:
;;   ok true if the withdrawal was accepted or rejected and removed from the
;;   pending list, ok false if it is still pending in sbtc-registry, or an
;;   error if the request-id is not tracked for this key.
;;
;; #[allow(unchecked_data)]
(define-public (settle-pending-withdrawal
        (staker principal)
        (signer-manager <reward-claim-signer-manager-trait>)
        (request-id uint)
    )
    (settle-pending-withdrawal-impl staker signer-manager request-id)
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
;;   ok with the number of items that resolved to accepted or rejected.
(define-public (settle-pending-withdrawals
        (signer-manager <reward-claim-signer-manager-trait>)
        (items (list 100 {
            staker: principal,
            request-id: uint,
        }))
    )
    (ok (get resolved
        (fold count-settlement items {
            signer-manager: signer-manager,
            resolved: u0,
        })
    ))
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
