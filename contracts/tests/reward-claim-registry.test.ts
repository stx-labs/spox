import { Cl, type ClarityValue, type OptionalCV, type UIntCV, privateKeyToAddress, cvToJSON } from "@stacks/transactions";
import { beforeEach, describe, expect, it } from "vitest";
import {
  ERR_ALREADY_CLAIMED,
  ERR_ALREADY_REGISTERED,
  ERR_INSUFFICIENT_FEE,
  ERR_INVALID_START_REWARD_CYCLE,
  ERR_NO_CURRENT_POSITION,
  ERR_NOT_ADMIN,
  ERR_NOT_REGISTERED,
  ERR_SIGNER_MANAGER_MISMATCH,
  ERR_UNKNOWN_PENDING_WITHDRAWAL,
  ERR_REENTRANT_CALL,
  ERR_UNAUTHORIZED,
  ERR_ZERO_FEE,
  FEE_PER_CLAIM,
  MALICIOUS_SIGNER_MANAGER,
  REENTER_ADD_CLAIMS,
  REENTER_CANCEL,
  REENTER_PROCESS_CLAIMS,
  REENTER_REGISTER,
  REENTER_SETTLE,
  addClaims,
  MOCK_SIGNER_MANAGER,
  SIGNER_MANAGER,
  SIGNER_PRIVATE_KEY,
  SIGNER_SET_MIN_USTX,
  acceptWithdrawal,
  bondPeriodToRewardCycle,
  currentDistributionCycle,
  currentRewardCycle,
  deployer,
  fundAndCalculateRewards,
  fundAndClaimSignerRewards,
  getPendingSettlements,
  getPendingClaims,
  getEarned,
  getMaliciousLastReenterError,
  getRegistration,
  initialNextClaimDistribution,
  initPox5,
  registerMaliciousSignerManager,
  mineUntilPastDistribution,
  processRewardClaim,
  registerForBond,
  registerForClaims,
  registerMockSignerManager,
  registerSignerManager,
  rejectWithdrawal,
  sbtcBalance,
  setMaliciousReenterMode,
  setMockClaimRewardsResult,
  setMockClaimStakerResult,
  setMockSettleResult,
  setupBond,
  stakeFor,
  stakeForMalicious,
  stakeForMock,
  stakeWithPoxAddr,
  stxBalance,
  wallet1,
  wallet2,
  wallet3,
} from "./pox-5-fixtures";


function settlePendingWithdrawal(staker: string, requestId: bigint, sender: string) {
  return simnet.callPublicFn(
    "reward-claim-registry",
    "settle-pending-withdrawal",
    [Cl.principal(staker), Cl.principal(SIGNER_MANAGER), Cl.uint(requestId)],
    sender,
  );
}

// Integration tests for reward-claim-registry against the REAL pox-5, signer-manager,
// and sBTC contracts. A staker must have a genuine pox-5 position under the
// signer-manager to be registerable, so registrations here are all real.
//
// STX-stake registrations starting at reward cycle 1 use next-claim-distribution=3
// (one full reward-cycle claim). Mine past that distribution before expecting pending
// claims / successful process-reward-claim.

const STX_START = 1n;
const STX_FIRST_CLAIM_DIST = initialNextClaimDistribution(STX_START, true); // 3

function stxRegistration(
  remaining: bigint,
  nextClaimDistribution: bigint,
  prepaid: bigint = remaining * FEE_PER_CLAIM,
) {
  return Cl.tuple({
    "bond-index": Cl.none(),
    "remaining-cycles": Cl.uint(remaining),
    "one-claim-per-reward-cycle": Cl.bool(true),
    "next-claim-distribution": Cl.uint(nextClaimDistribution),
    "prepaid-ustx": Cl.uint(prepaid),
  });
}

function bondRegistration(
  bondIndex: bigint,
  remaining: bigint,
  nextClaimDistribution: bigint,
  prepaid: bigint = remaining * FEE_PER_CLAIM,
) {
  return Cl.tuple({
    "bond-index": Cl.some(Cl.uint(bondIndex)),
    "remaining-cycles": Cl.uint(remaining),
    "one-claim-per-reward-cycle": Cl.bool(false),
    "next-claim-distribution": Cl.uint(nextClaimDistribution),
    "prepaid-ustx": Cl.uint(prepaid),
  });
}

function pendingRow(
  staker: string,
  rewardCycle: bigint,
  bondIndex: OptionalCV<UIntCV>,
) {
  return Cl.tuple({
    "signer-manager": Cl.principal(SIGNER_MANAGER),
    "staker": Cl.principal(staker),
    "bond-index": bondIndex,
    "reward-cycle": Cl.uint(rewardCycle),
  });
}

/** Expected `ok` payload from `get-pending-claims`: `{ rows, next }`. */
function pendingClaimsPage(
  rows: ReturnType<typeof pendingRow>[],
  next: ClarityValue = Cl.none(),
) {
  return Cl.tuple({
    rows: Cl.list(rows),
    next,
  });
}

function settlement(staker: string, requestIds: bigint[]) {
  return Cl.tuple({
    "staker": Cl.principal(staker),
    "signer-manager": Cl.principal(SIGNER_MANAGER),
    "request-ids": Cl.list(requestIds.map((id) => Cl.uint(id))),
  });
}

/** Register an STX-stake position and mine until its first claim is pending. */
function registerStxAndAdvance(staker: string, fee: bigint, sender = staker) {
  const result = registerForClaims(staker, fee, sender, SIGNER_MANAGER, STX_START, true);
  mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
  return result;
}

describe("admin", () => {
  it("deployer starts as admin", () => {
    expect(
      simnet.callReadOnlyFn("reward-claim-registry", "is-admin", [Cl.principal(deployer)], deployer)
        .result,
    ).toBeBool(true);
  });

  it("admin can grant and revoke other admins", () => {
    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "update-admin",
        [Cl.principal(wallet1), Cl.bool(true)],
        deployer,
      ).result,
    ).toBeOk(Cl.principal(wallet1));
    expect(
      simnet.callReadOnlyFn("reward-claim-registry", "is-admin", [Cl.principal(wallet1)], deployer)
        .result,
    ).toBeBool(true);

    simnet.callPublicFn(
      "reward-claim-registry",
      "update-admin",
      [Cl.principal(wallet1), Cl.bool(false)],
      wallet1,
    );
    expect(
      simnet.callReadOnlyFn("reward-claim-registry", "is-admin", [Cl.principal(wallet1)], deployer)
        .result,
    ).toBeBool(false);
  });

  it("non-admin cannot update admins", () => {
    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "update-admin",
        [Cl.principal(wallet2), Cl.bool(true)],
        wallet1,
      ).result,
    ).toBeErr(Cl.uint(ERR_NOT_ADMIN));
  });

  // Documents the self-lockout footgun: the sole admin can remove itself,
  // permanently bricking set-fee-per-cycle and update-admin. If a last-admin
  // guard is added, flip this expectation.
  it("the sole admin CAN lock itself out (no last-admin guard)", () => {
    simnet.callPublicFn(
      "reward-claim-registry",
      "update-admin",
      [Cl.principal(deployer), Cl.bool(false)],
      deployer,
    );
    expect(
      simnet.callPublicFn("reward-claim-registry", "set-fee-per-cycle", [Cl.uint(1)], deployer).result,
    ).toBeErr(Cl.uint(ERR_NOT_ADMIN));
  });
});

describe("set-fee-per-cycle", () => {
  it("admin can change the fee", () => {
    expect(
      simnet.callPublicFn("reward-claim-registry", "set-fee-per-cycle", [Cl.uint(250000)], deployer)
        .result,
    ).toBeOk(Cl.bool(true));
  });
  it("rejects a non-admin", () => {
    expect(
      simnet.callPublicFn("reward-claim-registry", "set-fee-per-cycle", [Cl.uint(250000)], wallet1)
        .result,
    ).toBeErr(Cl.uint(ERR_NOT_ADMIN));
  });
  it("rejects a zero fee", () => {
    expect(
      simnet.callPublicFn("reward-claim-registry", "set-fee-per-cycle", [Cl.uint(0)], deployer).result,
    ).toBeErr(Cl.uint(ERR_ZERO_FEE));
  });
});

describe("register-for-claims", () => {
  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    stakeFor(wallet1, SIGNER_SET_MIN_USTX, 2n);
  });

  it("registers a real staking position and returns claims bought", () => {
    const { result } = registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    expect(result).toBeOk(Cl.uint(3));
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(3n, STX_FIRST_CLAIM_DIST),
    );
  });

  it("escrows exactly the used portion of the fee", () => {
    const before = stxBalance(wallet1);
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM + FEE_PER_CLAIM / 2n, wallet1, SIGNER_MANAGER, STX_START, true);
    expect(before - stxBalance(wallet1)).toBe(3n * FEE_PER_CLAIM);
  });

  it("escrows nothing when an admin registers", () => {
    const before = stxBalance(deployer);
    const { result } = registerForClaims(wallet1, 3n * FEE_PER_CLAIM, deployer, SIGNER_MANAGER, STX_START, true);
    expect(result).toBeOk(Cl.uint(3));
    expect(stxBalance(deployer)).toBe(before);
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(3n, STX_FIRST_CLAIM_DIST, 0n),
    );
  });

  it("caps claims bought at MAX_DISTRIBUTION_CYCLES (192)", () => {
    const before = stxBalance(wallet1);
    const { result } = registerForClaims(wallet1, 500n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    expect(result).toBeOk(Cl.uint(192));
    expect(before - stxBalance(wallet1)).toBe(192n * FEE_PER_CLAIM);
  });

  it("rejects a fee too small to buy a single claim", () => {
    expect(registerForClaims(wallet1, FEE_PER_CLAIM - 1n, wallet1, SIGNER_MANAGER, STX_START, true).result).toBeErr(
      Cl.uint(ERR_INSUFFICIENT_FEE),
    );
  });

  it("rejects a staker with no pox-5 position", () => {
    expect(registerForClaims(wallet3, FEE_PER_CLAIM, wallet3, SIGNER_MANAGER, STX_START, true).result).toBeErr(
      Cl.uint(ERR_NO_CURRENT_POSITION),
    );

    stakeFor(wallet3, SIGNER_SET_MIN_USTX, 2n);
    const { result } = registerForClaims(wallet3, FEE_PER_CLAIM, wallet3, SIGNER_MANAGER, STX_START, true);
    expect(result).toBeOk(Cl.uint(1));
  });

  it("rejects a duplicate registration", () => {
    const first = registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    expect(first.result).toBeOk(Cl.uint(1));

    expect(
      registerForClaims(wallet1, 2n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true).result,
    ).toBeErr(Cl.uint(ERR_ALREADY_REGISTERED));

    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(1n, STX_FIRST_CLAIM_DIST),
    );
  });

  it("rejects a signer-manager that does not match the staker's position", () => {
    registerMockSignerManager();
    expect(
      registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, MOCK_SIGNER_MANAGER, STX_START, true).result,
    ).toBeErr(Cl.uint(ERR_SIGNER_MANAGER_MISMATCH));
  });

  it("rejects a start-reward-cycle before the position's first-reward-cycle", () => {
    expect(
      registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, 0n, true).result,
    ).toBeErr(Cl.uint(ERR_INVALID_START_REWARD_CYCLE));
  });

  it("seeds the schedule from the caller-supplied start-reward-cycle", () => {
    const start = 2n;
    const { result } = registerForClaims(
      wallet1,
      FEE_PER_CLAIM,
      wallet1,
      SIGNER_MANAGER,
      start,
      true,
    );
    expect(result).toBeOk(Cl.uint(1));
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(1n, initialNextClaimDistribution(start, true)),
    );
  });

  it("rejects a third party who is not an admin", () => {
    expect(
      registerForClaims(wallet1, FEE_PER_CLAIM, wallet2, SIGNER_MANAGER, STX_START, true).result,
    ).toBeErr(Cl.uint(ERR_UNAUTHORIZED));
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeNone();
  });

  it("is not pending until next-claim-distribution has elapsed", () => {
    registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([pendingRow(wallet1, STX_START, Cl.none())]));
  });
});

describe("add-claims", () => {
  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    stakeFor(wallet1, SIGNER_SET_MIN_USTX, 2n);
  });

  it("adds installments to an existing registration", () => {
    registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    expect(addClaims(wallet1, 2n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.uint(2));
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(3n, STX_FIRST_CLAIM_DIST),
    );
  });

  it("preserves next-claim-distribution and cadence after a claim", () => {
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());

    const nextAfterClaim = STX_FIRST_CLAIM_DIST + 2n;
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(stxRegistration(2n, nextAfterClaim));

    expect(addClaims(wallet1, 2n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.uint(2));
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(stxRegistration(4n, nextAfterClaim));
  });

  it("rejects when no registration exists", () => {
    expect(addClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER).result).toBeErr(
      Cl.uint(ERR_NOT_REGISTERED),
    );
  });

  it("rejects a fee too small to buy a single claim", () => {
    registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    expect(addClaims(wallet1, FEE_PER_CLAIM - 1n, wallet1, SIGNER_MANAGER).result).toBeErr(
      Cl.uint(ERR_INSUFFICIENT_FEE),
    );
  });

  it("rejects a third party who is not an admin", () => {
    registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    expect(addClaims(wallet1, 2n * FEE_PER_CLAIM, wallet2, SIGNER_MANAGER).result).toBeErr(
      Cl.uint(ERR_UNAUTHORIZED),
    );
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(1n, STX_FIRST_CLAIM_DIST),
    );
  });

  it("lets an admin add claims for a staker", () => {
    registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    const before = stxBalance(deployer);
    expect(addClaims(wallet1, 2n * FEE_PER_CLAIM, deployer, SIGNER_MANAGER).result).toBeOk(Cl.uint(2));
    expect(stxBalance(deployer)).toBe(before);
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(3n, STX_FIRST_CLAIM_DIST, FEE_PER_CLAIM),
    );
  });
});

describe("cancel-registration", () => {
  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    stakeFor(wallet1, SIGNER_SET_MIN_USTX, 2n);
  });

  it("lets the staker delete their registration and refunds escrow", () => {
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(3n, STX_FIRST_CLAIM_DIST),
    );

    const before = stxBalance(wallet1);
    const { result } = simnet.callPublicFn(
      "reward-claim-registry",
      "cancel-registration",
      [Cl.principal(wallet1), Cl.principal(SIGNER_MANAGER)],
      wallet1,
    );
    expect(result).toBeOk(Cl.uint(3n * FEE_PER_CLAIM));
    expect(stxBalance(wallet1) - before).toBe(3n * FEE_PER_CLAIM);
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeNone();
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));
  });

  it("refunds remaining escrow after a claim has burned one installment", () => {
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(2n, STX_FIRST_CLAIM_DIST + 2n),
    );

    const before = stxBalance(wallet1);
    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "cancel-registration",
        [Cl.principal(wallet1), Cl.principal(SIGNER_MANAGER)],
        wallet1,
      ).result,
    ).toBeOk(Cl.uint(2n * FEE_PER_CLAIM));
    expect(stxBalance(wallet1) - before).toBe(2n * FEE_PER_CLAIM);
  });

  it("rejects a caller who is not the staker", () => {
    registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "cancel-registration",
        [Cl.principal(wallet1), Cl.principal(SIGNER_MANAGER)],
        wallet2,
      ).result,
    ).toBeErr(Cl.uint(ERR_UNAUTHORIZED));
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(1n, STX_FIRST_CLAIM_DIST),
    );
  });

  it("rejects an admin canceling on behalf of a staker they registered", () => {
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, deployer, SIGNER_MANAGER, STX_START, true);
    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "cancel-registration",
        [Cl.principal(wallet1), Cl.principal(SIGNER_MANAGER)],
        deployer,
      ).result,
    ).toBeErr(Cl.uint(ERR_UNAUTHORIZED));
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(3n, STX_FIRST_CLAIM_DIST, 0n),
    );
  });

  it("rejects when no registration exists", () => {
    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "cancel-registration",
        [Cl.principal(wallet1), Cl.principal(SIGNER_MANAGER)],
        wallet1,
      ).result,
    ).toBeErr(Cl.uint(ERR_NOT_REGISTERED));
  });

  it("leaves pending L1 withdrawals settleable after cancel", () => {
    stakeWithPoxAddr(wallet2, SIGNER_SET_MIN_USTX, 2n, 100n);
    fundAndClaimSignerRewards(2000n, 1n);
    registerForClaims(wallet2, 3n * FEE_PER_CLAIM, wallet2, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);

    expect(processRewardClaim(wallet2, wallet2, SIGNER_MANAGER).result).toBeOk(Cl.some(Cl.uint(1)));
    expect(getPendingSettlements()).toBeOk(Cl.list([settlement(wallet2, [1n])]));

    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "cancel-registration",
        [Cl.principal(wallet2), Cl.principal(SIGNER_MANAGER)],
        wallet2,
      ).result,
    ).toBeOk(Cl.uint(2n * FEE_PER_CLAIM));
    expect(getRegistration(wallet2, SIGNER_MANAGER)).toBeNone();
    expect(getPendingSettlements()).toBeOk(Cl.list([settlement(wallet2, [1n])]));

    acceptWithdrawal(1n, 30n);
    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "settle-pending-withdrawal",
        [Cl.principal(wallet2), Cl.principal(SIGNER_MANAGER), Cl.uint(1)],
        wallet3,
      ).result,
    ).toBeOk(Cl.bool(true));
    expect(getPendingSettlements()).toBeOk(Cl.list([]));
  });
});

// Pins the STX eligibility boundary against an off-by-one that would claim
// the current reward cycle before it has finished accruing.
//
// Reward cycle r owns distribution cycles 2r and 2r+1. STX registrations
// use step 2, so next-claim-distribution is always the second half (2r+1).
// Pending only when next-claim-distribution < current-distribution-cycle,
// which is equivalent to requiring the current distribution cycle landing
// in a later reward cycle.
describe("STX claim eligibility boundary (no off-by-one)", () => {
  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    stakeFor(wallet1, SIGNER_SET_MIN_USTX, 2n);
  });

  it("seeds next-claim-distribution on the second half of a reward cycle (odd)", () => {
    registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    expect(STX_FIRST_CLAIM_DIST % 2n).toBe(1n);
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(1n, STX_FIRST_CLAIM_DIST),
    );
    // 2r+1 for r=1 -> distribution 3 is the second half of reward cycle 1
    expect(STX_FIRST_CLAIM_DIST / 2n).toBe(STX_START);
  });

  it("rejects claiming in the second half while CD == next (same reward cycle still live)", () => {
    // fundAndClaimSignerRewards(cycle 1) lands at burn 150 -> CD=3, RC=1
    // (second half of reward cycle 1). Registering here seeds next=3.
    fundAndClaimSignerRewards(2000n, 1n);
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);

    expect(currentDistributionCycle()).toBe(STX_FIRST_CLAIM_DIST); // 3
    expect(currentRewardCycle()).toBe(STX_START); // 1
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(3n, STX_FIRST_CLAIM_DIST),
    );

    // next == CD, and floor(next/2) == current reward cycle: not accrued yet.
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeErr(
      Cl.uint(ERR_ALREADY_CLAIMED),
    );
    // registration untouched
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(3n, STX_FIRST_CLAIM_DIST),
    );
  });

  it("becomes pending only once CD advances past next (current reward cycle > claim's)", () => {
    fundAndClaimSignerRewards(2000n, 1n);
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    expect(currentDistributionCycle()).toBe(STX_FIRST_CLAIM_DIST);
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeErr(
      Cl.uint(ERR_ALREADY_CLAIMED),
    );

    // Enter first half of reward cycle 2: CD=4 > next=3, RC=2 > 1
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
    expect(currentDistributionCycle()).toBe(STX_FIRST_CLAIM_DIST + 1n);
    expect(currentRewardCycle()).toBe(STX_START + 1n);
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([pendingRow(wallet1, STX_START, Cl.none())]));

    const stakerBefore = sbtcBalance(wallet1);
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());
    expect(sbtcBalance(wallet1) - stakerBefore).toBeGreaterThan(0n);
    // stepped by 2 to the next second-half distribution (reward cycle 2)
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(2n, STX_FIRST_CLAIM_DIST + 2n),
    );
  });

  it("after a claim, the next second-half target is again blocked until that cycle ends", () => {
    fundAndClaimSignerRewards(2000n, 1n);
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());

    const nextAfter = STX_FIRST_CLAIM_DIST + 2n; // 5 = second half of reward cycle 2
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(2n, nextAfter),
    );

    // Sit in the second half of reward cycle 2 (CD=5 == next): still the live cycle.
    mineUntilPastDistribution(nextAfter - 1n); // CD becomes 5
    expect(currentDistributionCycle()).toBe(nextAfter);
    expect(currentRewardCycle()).toBe(2n);
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeErr(
      Cl.uint(ERR_ALREADY_CLAIMED),
    );

    // Past it (CD=6, RC=3): now claimable for reward cycle 2
    mineUntilPastDistribution(nextAfter);
    expect(currentDistributionCycle()).toBe(nextAfter + 1n);
    expect(currentRewardCycle()).toBe(3n);
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([pendingRow(wallet1, 2n, Cl.none())]));
  });
});

describe("get-pending-claims", () => {
  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    stakeFor(wallet1, SIGNER_SET_MIN_USTX, 2n);
  });

  it("is empty when nothing is registered", () => {
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));
  });

  it("lists a registration once its claim distribution has elapsed", () => {
    registerStxAndAdvance(wallet1, FEE_PER_CLAIM);
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([pendingRow(wallet1, STX_START, Cl.none())]));
  });

  it("walks multiple registrations in insertion order", () => {
    stakeFor(wallet2, SIGNER_SET_MIN_USTX, 2n);
    registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    registerForClaims(wallet2, FEE_PER_CLAIM, wallet2, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
    expect(getPendingClaims()).toBeOk(
      pendingClaimsPage([pendingRow(wallet1, STX_START, Cl.none()), pendingRow(wallet2, STX_START, Cl.none())]));
  });

  it("drops a registration from the pending list once it is claimed", () => {
    stakeFor(wallet2, SIGNER_SET_MIN_USTX, 2n);
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    registerForClaims(wallet2, 3n * FEE_PER_CLAIM, wallet2, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);

    expect(getPendingClaims()).toBeOk(pendingClaimsPage([pendingRow(wallet1, STX_START, Cl.none()), pendingRow(wallet2, STX_START, Cl.none())]));

    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());

    expect(getPendingClaims()).toBeOk(pendingClaimsPage([pendingRow(wallet2, STX_START, Cl.none())]));

    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(2n, STX_FIRST_CLAIM_DIST + 2n),
    );
    expect(getRegistration(wallet2, SIGNER_MANAGER)).toBeSome(
      stxRegistration(3n, STX_FIRST_CLAIM_DIST),
    );
  });

  it(
    "paginates past non-pending registrations across >100 nodes (Rust get_all style)",
    () => {
      // 220 registrations; 5 not-pending in [0,100) and 5 in [100,200).
      // PENDING_TICKS=100, so three pages: 95 + 95 + 20 pending rows.
      const TOTAL = 220;
      const NOT_PENDING_START = 50n; // next-claim-distribution=101, still far future
      const notPending = new Set([10, 30, 50, 70, 90, 110, 130, 150, 170, 190]);

      const stakers = Array.from({ length: TOTAL }, (_, i) => {
        const hex = (BigInt(i) + 1n).toString(16).padStart(64, "0") + "01";
        return privateKeyToAddress(hex, "testnet");
      });

      for (let i = 0; i < TOTAL; i++) {
        const staker = stakers[i]!;
        expect(
          simnet.transferSTX(SIGNER_SET_MIN_USTX + 1_000_000n, staker, deployer).result,
        ).toBeOk(Cl.bool(true));
        stakeFor(staker, SIGNER_SET_MIN_USTX, 2n);
        const start = notPending.has(i) ? NOT_PENDING_START : STX_START;
        expect(
          registerForClaims(staker, FEE_PER_CLAIM, deployer, SIGNER_MANAGER, start, true).result,
        ).toBeOk(Cl.uint(1));
      }

      mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);

      const expectedPending = stakers.filter((_, i) => !notPending.has(i));
      expect(expectedPending).toHaveLength(210);
      const notPendingStakers = new Set(
        [...notPending].map((i) => stakers[i]!),
      );

      // Same pagination loop as RewardClaimRegistry::get_all_pending_claims:
      // keep calling with `next` until it is none (empty rows alone are not stop).
      const all: string[] = [];
      let cursor: OptionalCV = Cl.none();
      const pageSizes: number[] = [];
      for (let guard = 0; guard < 10; guard++) {
        const json = cvToJSON(getPendingClaims(cursor));
        expect(json.success).toBe(true);
        const page = json.value.value as {
          rows: {
            value: Array<{
              value: { staker: { value: string }; "reward-cycle": { value: string } };
            }>;
          };
          next: {
            value: null | {
              value: { staker: { value: string }; "signer-manager": { value: string } };
            };
          };
        };

        const rowStakers = page.rows.value.map((row) => row.value.staker.value);
        pageSizes.push(rowStakers.length);
        for (const row of page.rows.value) {
          expect(row.value["reward-cycle"].value).toBe(STX_START.toString());
          expect(notPendingStakers.has(row.value.staker.value)).toBe(false);
        }
        all.push(...rowStakers);

        if (page.next.value === null) {
          break;
        }
        const nextKey = page.next.value.value;
        cursor = Cl.some(
          Cl.tuple({
            staker: Cl.principal(nextKey.staker.value),
            "signer-manager": Cl.principal(nextKey["signer-manager"].value),
          }),
        );
      }

      expect(pageSizes).toEqual([95, 95, 20]);
      expect(all).toEqual(expectedPending);
    },
    180_000,
  );
});

describe("process-reward-claim (direct sBTC payout)", () => {
  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    stakeFor(wallet1, SIGNER_SET_MIN_USTX, 2n);
  });

  it("pays the staker its earned sBTC and decrements the registration", () => {
    fundAndClaimSignerRewards(2000n, 1n);
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);

    expect(getPendingClaims()).toBeOk(pendingClaimsPage([pendingRow(wallet1, STX_START, Cl.none())]));
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(3n, STX_FIRST_CLAIM_DIST),
    );

    const stakerBefore = sbtcBalance(wallet1);
    const managerBefore = sbtcBalance(SIGNER_MANAGER);
    const { result } = processRewardClaim(wallet1, wallet2, SIGNER_MANAGER);
    expect(result).toBeOk(Cl.none());

    const paid = sbtcBalance(wallet1) - stakerBefore;
    expect(paid).toBeGreaterThan(0n);
    expect(managerBefore - sbtcBalance(SIGNER_MANAGER)).toBe(paid);

    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(2n, STX_FIRST_CLAIM_DIST + 2n),
    );
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));
  });

  it("pulls claim-rewards itself when the signer-manager hasn't, then pays", () => {
    fundAndCalculateRewards(2000n, 1n);
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
    expect(getEarned(SIGNER_MANAGER, 1n, Cl.none())).toBeGreaterThan(0n);

    const stakerBefore = sbtcBalance(wallet1);
    const { result } = processRewardClaim(wallet1, wallet2, SIGNER_MANAGER);
    expect(result).toBeOk(Cl.none());

    expect(getEarned(SIGNER_MANAGER, 1n, Cl.none())).toBe(0n);
    expect(sbtcBalance(wallet1) - stakerBefore).toBeGreaterThan(0n);

    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(2n, STX_FIRST_CLAIM_DIST + 2n),
    );
  });

  it("advances past a genuinely empty cycle without stalling", () => {
    registerStxAndAdvance(wallet1, 3n * FEE_PER_CLAIM);

    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(3n, STX_FIRST_CLAIM_DIST),
    );

    const { result } = processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    expect(result).toBeOk(Cl.none());

    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(2n, STX_FIRST_CLAIM_DIST + 2n),
    );
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));
  });

  it("errors for a staker with no registration", () => {
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeErr(Cl.uint(ERR_NOT_REGISTERED));
  });

  it("errors when the next claim distribution has not elapsed again", () => {
    registerStxAndAdvance(wallet1, 3n * FEE_PER_CLAIM);
    processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeErr(Cl.uint(ERR_ALREADY_CLAIMED));
  });

  it("deletes the registration on the final installment", () => {
    registerStxAndAdvance(wallet1, FEE_PER_CLAIM);
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(stxRegistration(1n, STX_FIRST_CLAIM_DIST));
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([pendingRow(wallet1, STX_START, Cl.none())]));

    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeNone();
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));
  });
});

describe("process-reward-claims (batch)", () => {
  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    stakeFor(wallet1, SIGNER_SET_MIN_USTX, 2n);
    stakeFor(wallet2, SIGNER_SET_MIN_USTX, 2n);
  });

  it("claims multiple stakers and returns the count", () => {
    fundAndClaimSignerRewards(2000n, 1n);
    registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    registerForClaims(wallet2, FEE_PER_CLAIM, wallet2, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);

    expect(getPendingClaims()).toBeOk(pendingClaimsPage([pendingRow(wallet1, STX_START, Cl.none()), pendingRow(wallet2, STX_START, Cl.none())]));

    const { result } = simnet.callPublicFn(
      "reward-claim-registry",
      "process-reward-claims",
      [Cl.principal(SIGNER_MANAGER), Cl.list([Cl.principal(wallet1), Cl.principal(wallet2)])],
      wallet3,
    );
    expect(result).toBeOk(Cl.uint(2));
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));
  });

  it("self-pulls once for the batch when claim-rewards wasn't called", () => {
    fundAndCalculateRewards(2000n, 1n);
    registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    registerForClaims(wallet2, FEE_PER_CLAIM, wallet2, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
    expect(getEarned(SIGNER_MANAGER, 1n, Cl.none())).toBeGreaterThan(0n);

    const w1Before = sbtcBalance(wallet1);
    const w2Before = sbtcBalance(wallet2);

    const { result } = simnet.callPublicFn(
      "reward-claim-registry",
      "process-reward-claims",
      [Cl.principal(SIGNER_MANAGER), Cl.list([Cl.principal(wallet1), Cl.principal(wallet2)])],
      wallet3,
    );
    expect(result).toBeOk(Cl.uint(2));

    expect(getEarned(SIGNER_MANAGER, 1n, Cl.none())).toBe(0n);
    expect(sbtcBalance(wallet1) - w1Before).toBeGreaterThan(0n);
    expect(sbtcBalance(wallet2) - w2Before).toBeGreaterThan(0n);
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));
  });

  it("skips unregistered stakers without aborting the batch", () => {
    registerStxAndAdvance(wallet1, FEE_PER_CLAIM);

    expect(getPendingClaims()).toBeOk(pendingClaimsPage([pendingRow(wallet1, STX_START, Cl.none())]));

    const { result } = simnet.callPublicFn(
      "reward-claim-registry",
      "process-reward-claims",
      [Cl.principal(SIGNER_MANAGER), Cl.list([Cl.principal(wallet1), Cl.principal(wallet2)])],
      wallet3,
    );
    expect(result).toBeOk(Cl.uint(1));
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));
  });

  it("skips not-yet-pending stakers without aborting the batch", () => {
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    registerForClaims(wallet2, 3n * FEE_PER_CLAIM, wallet2, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);

    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([pendingRow(wallet2, STX_START, Cl.none())]));

    const { result } = simnet.callPublicFn(
      "reward-claim-registry",
      "process-reward-claims",
      [Cl.principal(SIGNER_MANAGER), Cl.list([Cl.principal(wallet1), Cl.principal(wallet2)])],
      wallet3,
    );
    expect(result).toBeOk(Cl.uint(1));
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      stxRegistration(2n, STX_FIRST_CLAIM_DIST + 2n),
    );
    expect(getRegistration(wallet2, SIGNER_MANAGER)).toBeSome(
      stxRegistration(2n, STX_FIRST_CLAIM_DIST + 2n),
    );
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));
  });
});

describe("bond path", () => {
  const BOND_INDEX = 0n;

  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    setupBond(BOND_INDEX, [wallet1], 100_000_000n);
    registerForBond(wallet1, BOND_INDEX, 5_000_000n);
  });

  it("get-position resolves the bond membership under the signer-manager", () => {
    const { result } = simnet.callReadOnlyFn(
      "reward-claim-registry",
      "get-position",
      [Cl.principal(wallet1)],
      deployer,
    );
    expect(result).toBeSome(
      Cl.tuple({
        signer: Cl.principal(SIGNER_MANAGER),
        "first-reward-cycle": Cl.uint(bondPeriodToRewardCycle(BOND_INDEX)),
        "bond-index": Cl.some(Cl.uint(BOND_INDEX)),
      }),
    );
  });

  it("registers a bond position and lists it as pending after its first half elapses", () => {
    const start = bondPeriodToRewardCycle(BOND_INDEX);
    const firstClaimDist = initialNextClaimDistribution(start, false);
    const { result } = registerForClaims(
      wallet1,
      3n * FEE_PER_CLAIM,
      wallet1,
      SIGNER_MANAGER,
      start,
      false,
    );
    expect(result).toBeOk(Cl.uint(3));
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));

    mineUntilPastDistribution(firstClaimDist);
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([pendingRow(wallet1, start, Cl.some(Cl.uint(BOND_INDEX)))]));
  });

  it("process-reward-claim drives the bond claim path (empty-cycle advance)", () => {
    const start = bondPeriodToRewardCycle(BOND_INDEX);
    const firstClaimDist = initialNextClaimDistribution(start, false);
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, start, false);
    mineUntilPastDistribution(firstClaimDist);
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      bondRegistration(BOND_INDEX, 2n, firstClaimDist + 1n),
    );
  });

  it("register-for-claims looks up the bond-index from pox-5", () => {
    const start = bondPeriodToRewardCycle(BOND_INDEX);
    expect(registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, start, false).result).toBeOk(
      Cl.uint(1),
    );
    expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
      bondRegistration(BOND_INDEX, 1n, initialNextClaimDistribution(start, false)),
    );
  });
});

describe("L1 withdrawal path + settlements", () => {
  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    stakeWithPoxAddr(wallet1, SIGNER_SET_MIN_USTX, 2n, 100n);
    fundAndClaimSignerRewards(2000n, 1n);
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
  });

  it("records a withdrawal request-id and lists it as a pending settlement", () => {
    const { result } = processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    expect(result).toBeOk(Cl.some(Cl.uint(1)));
    expect(getPendingSettlements()).toBeOk(Cl.list([settlement(wallet1, [1n])]));
  });

  it("settles an ACCEPTED withdrawal and clears it from the settlement list", () => {
    processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    acceptWithdrawal(1n, 30n);
    const { result } = settlePendingWithdrawal(wallet1, 1n, wallet2);
    expect(result).toBeOk(Cl.bool(true));
    expect(getPendingSettlements()).toBeOk(Cl.list([]));
  });

  it("settles a REJECTED withdrawal (reclaims to the staker) and clears it", () => {
    processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    rejectWithdrawal(1n);
    const { result } = settlePendingWithdrawal(wallet1, 1n, wallet2);
    expect(result).toBeOk(Cl.bool(true));
    expect(getPendingSettlements()).toBeOk(Cl.list([]));
  });

  it("is a no-op while the withdrawal is still pending", () => {
    processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    expect(settlePendingWithdrawal(wallet1, 1n, wallet2).result).toBeOk(Cl.bool(false));
    expect(getPendingSettlements()).toBeOk(Cl.list([settlement(wallet1, [1n])]));
  });

  it("errors on an unknown pending withdrawal", () => {
    processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    expect(settlePendingWithdrawal(wallet1, 9999n, wallet2).result).toBeErr(
      Cl.uint(ERR_UNKNOWN_PENDING_WITHDRAWAL),
    );
  });

  it("batch settle-pending-withdrawals resolves accepted items and counts them", () => {
    processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    acceptWithdrawal(1n, 30n);
    const { result } = simnet.callPublicFn(
      "reward-claim-registry",
      "settle-pending-withdrawals",
      [
        Cl.principal(SIGNER_MANAGER),
        Cl.list([
          Cl.tuple({ staker: Cl.principal(wallet1), "request-id": Cl.uint(1) }),
          Cl.tuple({ staker: Cl.principal(wallet1), "request-id": Cl.uint(9999) }),
        ]),
      ],
      wallet2,
    );
    expect(result).toBeOk(Cl.uint(1));
    expect(getPendingSettlements()).toBeOk(Cl.list([]));
  });
});

// Schedule / advance invariants. Several STX boundary cases are already
// covered above; this suite tests catch-up, bond cadence, the desired
// past-cycle bond skip, and mock SM error advance.
describe("claim schedule invariants", () => {
  describe("STX: catch-up and at-most-once per reward cycle", () => {
    beforeEach(() => {
      initPox5();
      registerSignerManager(SIGNER_PRIVATE_KEY);
      stakeFor(wallet1, SIGNER_SET_MIN_USTX, 6n);
    });

    it("can process multiple claims back-to-back when many distributions are past", () => {
      registerForClaims(wallet1, 5n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
      // Seed next=3; mine until CD > 7 so reward cycles 1, 2, and 3 are all claimable
      // without further mining between claims.
      mineUntilPastDistribution(STX_FIRST_CLAIM_DIST + 4n); // past dist 7
      expect(currentDistributionCycle()).toBeGreaterThan(STX_FIRST_CLAIM_DIST + 4n);

      expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());
      expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
        stxRegistration(4n, STX_FIRST_CLAIM_DIST + 2n),
      );

      expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());
      expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
        stxRegistration(3n, STX_FIRST_CLAIM_DIST + 4n),
      );

      expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());
      expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
        stxRegistration(2n, STX_FIRST_CLAIM_DIST + 6n),
      );
    });

    it("after an STX claim, the same reward cycle cannot be claimed again", () => {
      registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
      mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);

      const claimedRewardCycle = STX_START;
      expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());

      const next = STX_FIRST_CLAIM_DIST + 2n;
      expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(stxRegistration(2n, next));
      // next still encodes a later reward cycle; immediate re-claim is blocked
      expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeErr(
        Cl.uint(ERR_ALREADY_CLAIMED),
      );
      expect(next / 2n).toBe(claimedRewardCycle + 1n);
    });
  });

  describe("bond: once per distribution / at most twice per reward cycle", () => {
    const BOND_INDEX = 0n;

    beforeEach(() => {
      initPox5();
      registerSignerManager(SIGNER_PRIVATE_KEY);
      setupBond(BOND_INDEX, [wallet1], 100_000_000n);
      registerForBond(wallet1, BOND_INDEX, 5_000_000n);
    });

    it("claims at most once per distribution cycle and twice across one reward cycle", () => {
      const start = bondPeriodToRewardCycle(BOND_INDEX);
      const firstHalf = initialNextClaimDistribution(start, false); // 2*start
      const secondHalf = firstHalf + 1n;
      const bond = Cl.some(Cl.uint(BOND_INDEX));

      registerForClaims(wallet1, 4n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, start, false);
      mineUntilPastDistribution(firstHalf);

      expect(getPendingClaims()).toBeOk(pendingClaimsPage([pendingRow(wallet1, start, bond)]));
      expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());
      expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
        bondRegistration(BOND_INDEX, 3n, secondHalf),
      );
      // same distribution: not pending again until CD advances
      expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeErr(
        Cl.uint(ERR_ALREADY_CLAIMED),
      );

      expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));

      mineUntilPastDistribution(secondHalf);
      expect(getPendingClaims()).toBeOk(pendingClaimsPage([pendingRow(wallet1, start, bond)]));
      expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());
      expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
        bondRegistration(BOND_INDEX, 2n, secondHalf + 1n),
      );
      // two installments consumed for reward cycle `start`; next targets the following cycle
      expect((secondHalf + 1n) / 2n).toBe(start + 1n);
    });

    it("when the pending distribution is in a fully past reward cycle, claims only once for that cycle", () => {
      const start = bondPeriodToRewardCycle(BOND_INDEX);
      const firstHalf = initialNextClaimDistribution(start, false);
      const secondHalf = firstHalf + 1n;
      const nextCycleFirstHalf = firstHalf + 2n;

      registerForClaims(wallet1, 4n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, start, false);
      // Leave both halves of `start` in the past before the first claim.
      mineUntilPastDistribution(secondHalf);
      expect(currentDistributionCycle()).toBeGreaterThan(secondHalf);
      expect(currentRewardCycle()).toBeGreaterThan(start);

      expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());
      // Skip the second half of the already-finished reward cycle.
      expect(getRegistration(wallet1, SIGNER_MANAGER)).toBeSome(
        bondRegistration(BOND_INDEX, 3n, nextCycleFirstHalf),
      );
      expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeErr(
        Cl.uint(ERR_ALREADY_CLAIMED),
      );
    });
  });

  describe("signer-manager errors still advance remaining-cycles", () => {
    beforeEach(() => {
      initPox5();
      registerMockSignerManager();
      stakeForMock(wallet1, SIGNER_SET_MIN_USTX, 4n);
    });

    it("decrements when claim-rewards errors (get-earned > 0 pull path)", () => {
      fundAndCalculateRewards(2000n, 1n);
      expect(getEarned(MOCK_SIGNER_MANAGER, 1n, Cl.none())).toBeGreaterThan(0n);

      setMockClaimRewardsResult(true, 4242n);
      registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, MOCK_SIGNER_MANAGER, STX_START, true);
      mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);

      expect(getRegistration(wallet1, MOCK_SIGNER_MANAGER)).toBeSome(
        stxRegistration(3n, STX_FIRST_CLAIM_DIST),
      );
      expect(processRewardClaim(wallet1, wallet1, MOCK_SIGNER_MANAGER).result).toBeOk(Cl.none());
      expect(getRegistration(wallet1, MOCK_SIGNER_MANAGER)).toBeSome(
        stxRegistration(2n, STX_FIRST_CLAIM_DIST + 2n),
      );
      // pull did not succeed, so pox-5 still shows unpulled earned
      expect(getEarned(MOCK_SIGNER_MANAGER, 1n, Cl.none())).toBeGreaterThan(0n);
    });

    it("decrements when claim-staker-rewards errors", () => {
      setMockClaimStakerResult(true, 4242n);
      registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, MOCK_SIGNER_MANAGER, STX_START, true);
      mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);

      // No funding => get-earned == 0, so the registry skips claim-rewards and
      // hits claim-staker-rewards, which we force to err.
      expect(getEarned(MOCK_SIGNER_MANAGER, 1n, Cl.none())).toBe(0n);
      expect(processRewardClaim(wallet1, wallet1, MOCK_SIGNER_MANAGER).result).toBeOk(Cl.none());
      expect(getRegistration(wallet1, MOCK_SIGNER_MANAGER)).toBeSome(
        stxRegistration(2n, STX_FIRST_CLAIM_DIST + 2n),
      );
    });

    it("decrements when claim-rewards succeeds but claim-staker-rewards errors", () => {
      fundAndCalculateRewards(2000n, 1n);
      expect(getEarned(MOCK_SIGNER_MANAGER, 1n, Cl.none())).toBeGreaterThan(0n);

      setMockClaimRewardsResult(false);
      setMockClaimStakerResult(true, 4242n);
      registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, MOCK_SIGNER_MANAGER, STX_START, true);
      mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);

      expect(processRewardClaim(wallet1, wallet1, MOCK_SIGNER_MANAGER).result).toBeOk(Cl.none());
      expect(getRegistration(wallet1, MOCK_SIGNER_MANAGER)).toBeSome(
        stxRegistration(2n, STX_FIRST_CLAIM_DIST + 2n),
      );
    });
  });
});

// Clarity rejects calling the same public function already on the stack
// (CircularReference). The guard is for *cross-function* reentry: a signer-manager
// callback invoking process-reward-claims (private impl), cancel, or settle.
describe("reentrancy", () => {
  beforeEach(() => {
    initPox5();
    registerMaliciousSignerManager();
    stakeForMalicious(wallet1, SIGNER_SET_MIN_USTX, 4n);
    registerForClaims(
      wallet1,
      3n * FEE_PER_CLAIM,
      wallet1,
      MALICIOUS_SIGNER_MANAGER,
      STX_START,
      true,
    );
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
  });

  function expectSingleAdvance() {
    expect(getRegistration(wallet1, MALICIOUS_SIGNER_MANAGER)).toBeSome(
      stxRegistration(2n, STX_FIRST_CLAIM_DIST + 2n),
    );
  }

  it("blocks process-reward-claims reentry from claim-staker-rewards", () => {
    setMaliciousReenterMode(REENTER_PROCESS_CLAIMS, wallet1);
    expect(processRewardClaim(wallet1, wallet1, MALICIOUS_SIGNER_MANAGER).result).toBeOk(
      Cl.none(),
    );
    expect(getMaliciousLastReenterError()).toBeSome(Cl.uint(ERR_REENTRANT_CALL));
    expectSingleAdvance();
  });

  it("blocks cancel-registration reentry from claim-staker-rewards", () => {
    const before = stxBalance(wallet1);
    setMaliciousReenterMode(REENTER_CANCEL, wallet1);
    expect(processRewardClaim(wallet1, wallet1, MALICIOUS_SIGNER_MANAGER).result).toBeOk(
      Cl.none(),
    );
    expect(getMaliciousLastReenterError()).toBeSome(Cl.uint(ERR_REENTRANT_CALL));
    expectSingleAdvance();
    expect(stxBalance(wallet1)).toBe(before);
  });

  it("blocks settle-pending-withdrawal reentry from claim-staker-rewards", () => {
    setMaliciousReenterMode(REENTER_SETTLE, wallet1);
    expect(processRewardClaim(wallet1, wallet1, MALICIOUS_SIGNER_MANAGER).result).toBeOk(
      Cl.none(),
    );
    expect(getMaliciousLastReenterError()).toBeSome(Cl.uint(ERR_REENTRANT_CALL));
    expectSingleAdvance();
  });

  it("blocks process-reward-claims reentry from claim-rewards (pull path)", () => {
    fundAndCalculateRewards(2000n, 1n);
    expect(getEarned(MALICIOUS_SIGNER_MANAGER, 1n, Cl.none())).toBeGreaterThan(0n);

    setMaliciousReenterMode(REENTER_PROCESS_CLAIMS, wallet1);
    expect(processRewardClaim(wallet1, wallet1, MALICIOUS_SIGNER_MANAGER).result).toBeOk(
      Cl.none(),
    );
    expect(getMaliciousLastReenterError()).toBeSome(Cl.uint(ERR_REENTRANT_CALL));
    expectSingleAdvance();
  });

  it("rejects add-claims reentry via authorize-staker-or-admin, not the reentrancy gate", () => {
    setMaliciousReenterMode(REENTER_ADD_CLAIMS, wallet1);
    expect(processRewardClaim(wallet1, wallet1, MALICIOUS_SIGNER_MANAGER).result).toBeOk(
      Cl.none(),
    );
    expect(getMaliciousLastReenterError()).toBeSome(Cl.uint(ERR_UNAUTHORIZED));
    expectSingleAdvance();
  });

  it("rejects register-for-claims reentry via authorize-staker-or-admin, not the reentrancy gate", () => {
    setMaliciousReenterMode(REENTER_REGISTER, wallet1);
    expect(processRewardClaim(wallet1, wallet1, MALICIOUS_SIGNER_MANAGER).result).toBeOk(
      Cl.none(),
    );
    expect(getMaliciousLastReenterError()).toBeSome(Cl.uint(ERR_UNAUTHORIZED));
    expectSingleAdvance();
  });
});

describe("batch settle SM error does not stick the reentrancy lock", () => {
  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    registerMockSignerManager();
    stakeWithPoxAddr(wallet2, SIGNER_SET_MIN_USTX, 2n, 100n);
    stakeForMock(wallet3, SIGNER_SET_MIN_USTX, 4n);
    registerForClaims(wallet2, 3n * FEE_PER_CLAIM, wallet2, SIGNER_MANAGER, STX_START, true);
    registerForClaims(wallet3, 3n * FEE_PER_CLAIM, wallet3, MOCK_SIGNER_MANAGER, STX_START, true);
    fundAndClaimSignerRewards(2000n, 1n);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
  });

  it("batch settle-pending-withdrawals still allows later gated calls after the SM errors", () => {
    // Real L1 withdrawal so sbtc-registry has request-id 1 with an accepted status.
    expect(processRewardClaim(wallet2, wallet2, SIGNER_MANAGER).result).toBeOk(Cl.some(Cl.uint(1)));
    acceptWithdrawal(1n, 30n);

    // Mock SM tracks that same request-id so settle dispatches to the mock.
    setMockClaimStakerResult(false, 1001n, 1000n, Cl.some(Cl.uint(1)));
    expect(processRewardClaim(wallet3, wallet3, MOCK_SIGNER_MANAGER).result).toBeOk(
      Cl.some(Cl.uint(1)),
    );

    setMockSettleResult(true, 4242n);
    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "settle-pending-withdrawals",
        [
          Cl.principal(MOCK_SIGNER_MANAGER),
          Cl.list([
            Cl.tuple({ staker: Cl.principal(wallet3), "request-id": Cl.uint(1) }),
          ]),
        ],
        wallet2,
      ).result,
    ).toBeOk(Cl.uint(0));

    // Lock was cleared: the SM error surfaces, not ERR_REENTRANT_CALL.
    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "settle-pending-withdrawal",
        [Cl.principal(wallet3), Cl.principal(MOCK_SIGNER_MANAGER), Cl.uint(1)],
        wallet2,
      ).result,
    ).toBeErr(Cl.uint(4242));
    expect(processRewardClaim(wallet3, wallet3, MOCK_SIGNER_MANAGER).result).toBeErr(
      Cl.uint(ERR_ALREADY_CLAIMED),
    );

    setMockSettleResult(false);
    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "settle-pending-withdrawal",
        [Cl.principal(wallet3), Cl.principal(MOCK_SIGNER_MANAGER), Cl.uint(1)],
        wallet2,
      ).result,
    ).toBeOk(Cl.bool(true));
  });
});
