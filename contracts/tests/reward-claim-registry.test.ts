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
  burnHeight,
  getPendingWithdrawals,
  getPendingClaims,
  getRegistrations,
  getWithdrawals,
  getEarned,
  getMaliciousLastReenterError,
  getRegistration,
  initialNextClaimDistribution,
  initPox5,
  registerMaliciousSignerManager,
  mineUntilPastDistribution,
  mineUntilWithdrawalListable,
  processRewardClaim,
  registerForBond,
  registerForClaims,
  registerMockSignerManager,
  registerSignerManager,
  rejectWithdrawal,
  sbtcBalance,
  setMaliciousReenterMode,
  setMaliciousWithdrawalRequest,
  setMockClaimRewardsResult,
  setMockClaimStakerResult,
  settleAcceptedWithdrawalOnSignerManager,
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

function withdrawalRow(staker: string, requestId: bigint, signerManager = SIGNER_MANAGER) {
  return Cl.tuple({
    staker: Cl.principal(staker),
    "signer-manager": Cl.principal(signerManager),
    "request-id": Cl.uint(requestId),
  });
}

function pendingWithdrawalsPage(
  rows: ReturnType<typeof withdrawalRow>[],
  next: ClarityValue = Cl.none(),
) {
  return Cl.tuple({
    rows: Cl.list(rows),
    next,
  });
}

/** Full registration row from `get-registrations`: map value merged with key. */
function registrationRow(
  staker: string,
  remaining: bigint,
  nextClaimDistribution: bigint,
  prepaid: bigint = remaining * FEE_PER_CLAIM,
  oneClaimPerRewardCycle = true,
  bondIndex: OptionalCV<UIntCV> = Cl.none(),
) {
  return Cl.tuple({
    staker: Cl.principal(staker),
    "signer-manager": Cl.principal(SIGNER_MANAGER),
    "bond-index": bondIndex,
    "remaining-cycles": Cl.uint(remaining),
    "one-claim-per-reward-cycle": Cl.bool(oneClaimPerRewardCycle),
    "next-claim-distribution": Cl.uint(nextClaimDistribution),
    "prepaid-ustx": Cl.uint(prepaid),
  });
}

function registrationsPage(
  rows: ReturnType<typeof registrationRow>[],
  next: ClarityValue = Cl.none(),
) {
  return Cl.tuple({
    rows: Cl.list(rows),
    next,
  });
}

/** Full withdrawal row from `get-withdrawals`: key merged with indexed-height. */
function withdrawalEntryRow(
  staker: string,
  requestId: bigint,
  indexedHeight: bigint,
  signerManager = SIGNER_MANAGER,
) {
  return Cl.tuple({
    staker: Cl.principal(staker),
    "signer-manager": Cl.principal(signerManager),
    "request-id": Cl.uint(requestId),
    "indexed-height": Cl.uint(indexedHeight),
  });
}

function withdrawalsPage(
  rows: ReturnType<typeof withdrawalEntryRow>[],
  next: ClarityValue = Cl.none(),
) {
  return Cl.tuple({
    rows: Cl.list(rows),
    next,
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

    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "cancel-registration",
        [Cl.principal(wallet2), Cl.principal(SIGNER_MANAGER)],
        wallet2,
      ).result,
    ).toBeOk(Cl.uint(2n * FEE_PER_CLAIM));
    expect(getRegistration(wallet2, SIGNER_MANAGER)).toBeNone();

    acceptWithdrawal(1n, 30n);
    mineUntilWithdrawalListable();
    expect(getPendingWithdrawals()).toBeOk(
      pendingWithdrawalsPage([withdrawalRow(wallet2, 1n)]),
    );
    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "settle-pending-withdrawal",
        [Cl.principal(wallet2), Cl.principal(SIGNER_MANAGER), Cl.uint(1)],
        wallet3,
      ).result,
    ).toBeOk(Cl.bool(true));
    expect(getPendingWithdrawals()).toBeOk(pendingWithdrawalsPage([]));
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

describe("get-registrations", () => {
  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    stakeFor(wallet1, SIGNER_SET_MIN_USTX, 2n);
  });

  it("is empty when nothing is registered", () => {
    expect(getRegistrations()).toBeOk(registrationsPage([]));
  });

  it("lists a registration before its claim is pending", () => {
    registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));
    expect(getRegistrations()).toBeOk(
      registrationsPage([registrationRow(wallet1, 1n, STX_FIRST_CLAIM_DIST)]),
    );
  });

  it("walks multiple registrations in insertion order", () => {
    stakeFor(wallet2, SIGNER_SET_MIN_USTX, 2n);
    registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    registerForClaims(wallet2, 2n * FEE_PER_CLAIM, wallet2, SIGNER_MANAGER, STX_START, true);
    expect(getRegistrations()).toBeOk(
      registrationsPage([
        registrationRow(wallet1, 1n, STX_FIRST_CLAIM_DIST),
        registrationRow(wallet2, 2n, STX_FIRST_CLAIM_DIST),
      ]),
    );
  });

  it("keeps a registration after it is claimed (unlike get-pending-claims)", () => {
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.none());

    expect(getPendingClaims()).toBeOk(pendingClaimsPage([]));
    expect(getRegistrations()).toBeOk(
      registrationsPage([registrationRow(wallet1, 2n, STX_FIRST_CLAIM_DIST + 2n)]),
    );
  });

  it("drops a registration after cancel", () => {
    stakeFor(wallet2, SIGNER_SET_MIN_USTX, 2n);
    registerForClaims(wallet1, FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    registerForClaims(wallet2, FEE_PER_CLAIM, wallet2, SIGNER_MANAGER, STX_START, true);

    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "cancel-registration",
        [Cl.principal(wallet1), Cl.principal(SIGNER_MANAGER)],
        wallet1,
      ).result,
    ).toBeOk(Cl.uint(FEE_PER_CLAIM));

    expect(getRegistrations()).toBeOk(
      registrationsPage([registrationRow(wallet2, 1n, STX_FIRST_CLAIM_DIST)]),
    );
  });

  it(
    "paginates every registration across >100 nodes",
    () => {
      const TOTAL = 220;

      const stakers = Array.from({ length: TOTAL }, (_, i) => {
        const hex = (BigInt(i) + 1n).toString(16).padStart(64, "0") + "01";
        return privateKeyToAddress(hex, "testnet");
      });

      for (const staker of stakers) {
        expect(
          simnet.transferSTX(SIGNER_SET_MIN_USTX + 1_000_000n, staker, deployer).result,
        ).toBeOk(Cl.bool(true));
        stakeFor(staker, SIGNER_SET_MIN_USTX, 2n);
        expect(
          registerForClaims(staker, FEE_PER_CLAIM, deployer, SIGNER_MANAGER, STX_START, true)
            .result,
        ).toBeOk(Cl.uint(1));
      }

      const all: string[] = [];
      let cursor: OptionalCV = Cl.none();
      const pageSizes: number[] = [];
      for (let guard = 0; guard < 10; guard++) {
        const json = cvToJSON(getRegistrations(cursor));
        expect(json.success).toBe(true);
        const page = json.value.value as {
          rows: {
            value: Array<{
              value: {
                staker: { value: string };
                "remaining-cycles": { value: string };
                "next-claim-distribution": { value: string };
              };
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
          expect(row.value["remaining-cycles"].value).toBe("1");
          expect(row.value["next-claim-distribution"].value).toBe(
            STX_FIRST_CLAIM_DIST.toString(),
          );
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

      expect(pageSizes).toEqual([100, 100, 20]);
      expect(all).toEqual(stakers);
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

describe("L1 withdrawal path + pending withdrawals", () => {
  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    stakeWithPoxAddr(wallet1, SIGNER_SET_MIN_USTX, 2n, 100n);
    stakeWithPoxAddr(wallet2, SIGNER_SET_MIN_USTX, 2n, 100n);
    fundAndClaimSignerRewards(2000n, 1n);
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    registerForClaims(wallet2, 3n * FEE_PER_CLAIM, wallet2, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
  });

  it("records a withdrawal request-id and lists it once accepted and old enough", () => {
    const { result } = processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    expect(result).toBeOk(Cl.some(Cl.uint(1)));
    expect(getPendingWithdrawals()).toBeOk(pendingWithdrawalsPage([]));

    acceptWithdrawal(1n, 30n);
    expect(getPendingWithdrawals()).toBeOk(pendingWithdrawalsPage([]));

    mineUntilWithdrawalListable();
    expect(getPendingWithdrawals()).toBeOk(
      pendingWithdrawalsPage([withdrawalRow(wallet1, 1n)]),
    );
  });

  it("does not list a still-pending withdrawal even after the age gate", () => {
    processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    mineUntilWithdrawalListable();
    expect(getPendingWithdrawals()).toBeOk(pendingWithdrawalsPage([]));
  });

  it("settles an ACCEPTED withdrawal and clears it from the pending-withdrawal list", () => {
    processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    acceptWithdrawal(1n, 30n);
    const { result } = settlePendingWithdrawal(wallet1, 1n, wallet2);
    expect(result).toBeOk(Cl.bool(true));
    mineUntilWithdrawalListable();
    expect(getPendingWithdrawals()).toBeOk(pendingWithdrawalsPage([]));
  });

  it("settles a REJECTED withdrawal (reclaims to the staker) and clears it", () => {
    processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    rejectWithdrawal(1n);
    const { result } = settlePendingWithdrawal(wallet1, 1n, wallet2);
    expect(result).toBeOk(Cl.bool(true));
    mineUntilWithdrawalListable();
    expect(getPendingWithdrawals()).toBeOk(pendingWithdrawalsPage([]));
  });

  it("is a no-op while the withdrawal is still pending", () => {
    processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    expect(settlePendingWithdrawal(wallet1, 1n, wallet2).result).toBeOk(Cl.bool(false));
    mineUntilWithdrawalListable();
    expect(getPendingWithdrawals()).toBeOk(pendingWithdrawalsPage([]));
  });

  it("errors on an unknown pending withdrawal", () => {
    processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    expect(settlePendingWithdrawal(wallet1, 9999n, wallet2).result).toBeErr(
      Cl.uint(ERR_UNKNOWN_PENDING_WITHDRAWAL),
    );
  });

  it("prunes a registry id after the signer-manager already settled it", () => {
    processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    acceptWithdrawal(1n, 30n);
    expect(settleAcceptedWithdrawalOnSignerManager(1n, wallet2).result).toBeOk(Cl.bool(true));
    mineUntilWithdrawalListable();
    expect(getPendingWithdrawals()).toBeOk(
      pendingWithdrawalsPage([withdrawalRow(wallet1, 1n)]),
    );

    expect(settlePendingWithdrawal(wallet1, 1n, wallet2).result).toBeOk(Cl.bool(true));
    expect(getPendingWithdrawals()).toBeOk(pendingWithdrawalsPage([]));
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
    mineUntilWithdrawalListable();
    expect(getPendingWithdrawals()).toBeOk(pendingWithdrawalsPage([]));
  });

  it("process-reward-claims records withdrawal request-ids", () => {
    const { result } = simnet.callPublicFn(
      "reward-claim-registry",
      "process-reward-claims",
      [Cl.principal(SIGNER_MANAGER), Cl.list([Cl.principal(wallet1), Cl.principal(wallet2)])],
      wallet3,
    );
    expect(result).toBeOk(Cl.uint(2));

    acceptWithdrawal(1n, 30n);
    acceptWithdrawal(2n, 30n);
    mineUntilWithdrawalListable();
    expect(getPendingWithdrawals()).toBeOk(
      pendingWithdrawalsPage([withdrawalRow(wallet1, 1n), withdrawalRow(wallet2, 2n)]),
    );
  });
});

describe("get-pending-withdrawals pagination", () => {
  it(
    "paginates past not-yet-settleable withdrawals across >100 ids (Rust get_all style)",
    () => {
      // 150 stakers on pending-withdrawal-ll; 70 of them get a second claim so
      // there are 220 withdrawal IDs (one LL node each). 10 stakers stay
      // status-none: 5 of those first-claim nodes sit in the first 100 visits
      // and 5 in the next 50. Dual-claim ids append after the 150 first-claims.
      // PENDING_TICKS=100, so three pages of settleable rows: 95 + 93 + 19.
      const TOTAL_STAKERS = 150;
      const DUAL_CLAIM_COUNT = 70; // 150 + 70 = 220 withdrawal IDs
      const notSettleable = new Set([10, 30, 50, 70, 90, 110, 130, 140, 145, 149]);

      initPox5();
      registerSignerManager(SIGNER_PRIVATE_KEY);

      const stakers = Array.from({ length: TOTAL_STAKERS }, (_, i) => {
        const hex = (BigInt(i) + 1n).toString(16).padStart(64, "0") + "01";
        return privateKeyToAddress(hex, "testnet");
      });

      for (let i = 0; i < TOTAL_STAKERS; i++) {
        const staker = stakers[i]!;
        expect(
          simnet.transferSTX(SIGNER_SET_MIN_USTX + 1_000_000n, staker, deployer).result,
        ).toBeOk(Cl.bool(true));
        stakeWithPoxAddr(staker, SIGNER_SET_MIN_USTX, 6n, 100n);
        expect(
          registerForClaims(
            staker,
            3n * FEE_PER_CLAIM,
            deployer,
            SIGNER_MANAGER,
            STX_START,
            true,
          ).result,
        ).toBeOk(Cl.uint(3));
      }

      fundAndClaimSignerRewards(500_000n, 1n);
      mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);

      const idsByStaker = new Map<string, bigint[]>();
      const readWithdrawalId = (result: ClarityValue): bigint => {
        const json = cvToJSON(result) as {
          success: boolean;
          value: { value: { value: string } };
        };
        expect(json.success).toBe(true);
        return BigInt(json.value.value.value);
      };

      for (const staker of stakers) {
        const { result } = processRewardClaim(staker, deployer, SIGNER_MANAGER);
        idsByStaker.set(staker, [readWithdrawalId(result)]);
      }

      fundAndClaimSignerRewards(500_000n, 2n);
      mineUntilPastDistribution(STX_FIRST_CLAIM_DIST + 2n);

      for (let i = 0; i < DUAL_CLAIM_COUNT; i++) {
        const staker = stakers[i]!;
        const { result } = processRewardClaim(staker, deployer, SIGNER_MANAGER);
        idsByStaker.get(staker)!.push(readWithdrawalId(result));
      }

      const allIds = [...idsByStaker.values()].flat();
      expect(allIds).toHaveLength(220);
      expect(new Set(allIds).size).toBe(220);

      for (let i = 0; i < TOTAL_STAKERS; i++) {
        if (notSettleable.has(i)) continue;
        for (const id of idsByStaker.get(stakers[i]!)!) {
          expect(rejectWithdrawal(id).result).toBeOk(Cl.bool(true));
        }
      }

      mineUntilWithdrawalListable();

      const expectedIds: bigint[] = [];
      for (let i = 0; i < TOTAL_STAKERS; i++) {
        if (!notSettleable.has(i)) expectedIds.push(idsByStaker.get(stakers[i]!)![0]!);
      }
      for (let i = 0; i < DUAL_CLAIM_COUNT; i++) {
        if (!notSettleable.has(i)) expectedIds.push(idsByStaker.get(stakers[i]!)![1]!);
      }
      expect(expectedIds).toHaveLength(207);

      const notSettleableStakers = new Set(
        [...notSettleable].map((i) => stakers[i]!),
      );

      const listedIds: bigint[] = [];
      let cursor: OptionalCV = Cl.none();
      const pageSizes: number[] = [];
      for (let guard = 0; guard < 10; guard++) {
        const json = cvToJSON(getPendingWithdrawals(cursor));
        expect(json.success).toBe(true);
        const page = json.value.value as {
          rows: {
            value: Array<{
              value: {
                staker: { value: string };
                "request-id": { value: string };
              };
            }>;
          };
          next: {
            value: null | {
              value: {
                staker: { value: string };
                "signer-manager": { value: string };
                "request-id": { value: string };
              };
            };
          };
        };

        pageSizes.push(page.rows.value.length);
        for (const row of page.rows.value) {
          expect(notSettleableStakers.has(row.value.staker.value)).toBe(false);
          listedIds.push(BigInt(row.value["request-id"].value));
        }

        if (page.next.value === null) {
          break;
        }
        const nextKey = page.next.value.value;
        cursor = Cl.some(
          Cl.tuple({
            staker: Cl.principal(nextKey.staker.value),
            "signer-manager": Cl.principal(nextKey["signer-manager"].value),
            "request-id": Cl.uint(BigInt(nextKey["request-id"].value)),
          }),
        );
      }

      expect(pageSizes).toEqual([95, 93, 19]);
      expect(listedIds).toEqual(expectedIds);
    },
    300_000,
  );

  it(
    "returns empty rows with next set when the first 100 nodes are not settleable",
    () => {
      const TOTAL = 101;
      initPox5();
      registerSignerManager(SIGNER_PRIVATE_KEY);

      const stakers = Array.from({ length: TOTAL }, (_, i) => {
        const hex = (BigInt(i) + 1n).toString(16).padStart(64, "0") + "01";
        return privateKeyToAddress(hex, "testnet");
      });

      for (const staker of stakers) {
        expect(
          simnet.transferSTX(SIGNER_SET_MIN_USTX + 1_000_000n, staker, deployer).result,
        ).toBeOk(Cl.bool(true));
        stakeWithPoxAddr(staker, SIGNER_SET_MIN_USTX, 2n, 100n);
        expect(
          registerForClaims(staker, FEE_PER_CLAIM, deployer, SIGNER_MANAGER, STX_START, true)
            .result,
        ).toBeOk(Cl.uint(1));
      }

      fundAndClaimSignerRewards(500_000n, 1n);
      mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);

      for (let i = 0; i < TOTAL; i++) {
        expect(processRewardClaim(stakers[i]!, deployer, SIGNER_MANAGER).result).toBeOk(
          Cl.some(Cl.uint(BigInt(i + 1))),
        );
      }

      const lastId = BigInt(TOTAL);
      const lastStaker = stakers[TOTAL - 1]!;
      expect(acceptWithdrawal(lastId, 30n).result).toBeOk(Cl.bool(true));
      mineUntilWithdrawalListable();

      const first = cvToJSON(getPendingWithdrawals()) as {
        success: boolean;
        value: {
          value: {
            rows: { value: unknown[] };
            next: {
              value: null | {
                value: {
                  staker: { value: string };
                  "signer-manager": { value: string };
                  "request-id": { value: string };
                };
              };
            };
          };
        };
      };
      expect(first.success).toBe(true);
      expect(first.value.value.rows.value).toHaveLength(0);
      expect(first.value.value.next.value).not.toBeNull();

      const nextKey = first.value.value.next.value!.value;
      const second = getPendingWithdrawals(
        Cl.some(
          Cl.tuple({
            staker: Cl.principal(nextKey.staker.value),
            "signer-manager": Cl.principal(nextKey["signer-manager"].value),
            "request-id": Cl.uint(BigInt(nextKey["request-id"].value)),
          }),
        ),
      );
      expect(second).toBeOk(pendingWithdrawalsPage([withdrawalRow(lastStaker, lastId)]));
    },
    300_000,
  );
});

describe("get-withdrawals", () => {
  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    stakeWithPoxAddr(wallet1, SIGNER_SET_MIN_USTX, 2n, 100n);
    stakeWithPoxAddr(wallet2, SIGNER_SET_MIN_USTX, 2n, 100n);
    fundAndClaimSignerRewards(2000n, 1n);
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    registerForClaims(wallet2, 3n * FEE_PER_CLAIM, wallet2, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
  });

  it("is empty when nothing is indexed", () => {
    expect(getWithdrawals()).toBeOk(withdrawalsPage([]));
  });

  it("lists an indexed withdrawal immediately, before it is settleable", () => {
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.some(Cl.uint(1)));
    const indexedHeight = burnHeight();

    expect(getPendingWithdrawals()).toBeOk(pendingWithdrawalsPage([]));
    expect(getWithdrawals()).toBeOk(
      withdrawalsPage([withdrawalEntryRow(wallet1, 1n, indexedHeight)]),
    );
  });

  it("lists still-pending withdrawals even after the age gate", () => {
    processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    const indexedHeight = burnHeight();
    mineUntilWithdrawalListable();

    expect(getPendingWithdrawals()).toBeOk(pendingWithdrawalsPage([]));
    expect(getWithdrawals()).toBeOk(
      withdrawalsPage([withdrawalEntryRow(wallet1, 1n, indexedHeight)]),
    );
  });

  it("walks multiple withdrawals in insertion order", () => {
    processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    const height1 = burnHeight();
    processRewardClaim(wallet2, wallet2, SIGNER_MANAGER);
    const height2 = burnHeight();

    expect(getWithdrawals()).toBeOk(
      withdrawalsPage([
        withdrawalEntryRow(wallet1, 1n, height1),
        withdrawalEntryRow(wallet2, 2n, height2),
      ]),
    );
  });

  it("drops a withdrawal after it is settled", () => {
    processRewardClaim(wallet1, wallet1, SIGNER_MANAGER);
    processRewardClaim(wallet2, wallet2, SIGNER_MANAGER);
    const height2 = burnHeight();
    rejectWithdrawal(1n);
    expect(settlePendingWithdrawal(wallet1, 1n, deployer).result).toBeOk(Cl.bool(true));

    expect(getWithdrawals()).toBeOk(
      withdrawalsPage([withdrawalEntryRow(wallet2, 2n, height2)]),
    );
  });
});

describe("get-withdrawals pagination", () => {
  it(
    "paginates every indexed withdrawal across >100 nodes",
    () => {
      const TOTAL = 220;
      initPox5();
      registerSignerManager(SIGNER_PRIVATE_KEY);

      const stakers = Array.from({ length: TOTAL }, (_, i) => {
        const hex = (BigInt(i) + 1n).toString(16).padStart(64, "0") + "01";
        return privateKeyToAddress(hex, "testnet");
      });

      for (const staker of stakers) {
        expect(
          simnet.transferSTX(SIGNER_SET_MIN_USTX + 1_000_000n, staker, deployer).result,
        ).toBeOk(Cl.bool(true));
        stakeWithPoxAddr(staker, SIGNER_SET_MIN_USTX, 2n, 100n);
        expect(
          registerForClaims(staker, FEE_PER_CLAIM, deployer, SIGNER_MANAGER, STX_START, true)
            .result,
        ).toBeOk(Cl.uint(1));
      }

      fundAndClaimSignerRewards(500_000n, 1n);
      mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);

      const expected: { staker: string; requestId: bigint; indexedHeight: bigint }[] = [];
      for (let i = 0; i < TOTAL; i++) {
        const staker = stakers[i]!;
        const { result } = processRewardClaim(staker, deployer, SIGNER_MANAGER);
        expect(result).toBeOk(Cl.some(Cl.uint(BigInt(i + 1))));
        expected.push({
          staker,
          requestId: BigInt(i + 1),
          indexedHeight: burnHeight(),
        });
      }

      const listed: { staker: string; requestId: bigint; indexedHeight: bigint }[] = [];
      let cursor: OptionalCV = Cl.none();
      const pageSizes: number[] = [];
      for (let guard = 0; guard < 10; guard++) {
        const json = cvToJSON(getWithdrawals(cursor));
        expect(json.success).toBe(true);
        const page = json.value.value as {
          rows: {
            value: Array<{
              value: {
                staker: { value: string };
                "request-id": { value: string };
                "indexed-height": { value: string };
              };
            }>;
          };
          next: {
            value: null | {
              value: {
                staker: { value: string };
                "signer-manager": { value: string };
                "request-id": { value: string };
              };
            };
          };
        };

        pageSizes.push(page.rows.value.length);
        for (const row of page.rows.value) {
          listed.push({
            staker: row.value.staker.value,
            requestId: BigInt(row.value["request-id"].value),
            indexedHeight: BigInt(row.value["indexed-height"].value),
          });
        }

        if (page.next.value === null) {
          break;
        }
        const nextKey = page.next.value.value;
        cursor = Cl.some(
          Cl.tuple({
            staker: Cl.principal(nextKey.staker.value),
            "signer-manager": Cl.principal(nextKey["signer-manager"].value),
            "request-id": Cl.uint(BigInt(nextKey["request-id"].value)),
          }),
        );
      }

      expect(pageSizes).toEqual([100, 100, 20]);
      expect(listed).toEqual(expected);
    },
    300_000,
  );
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

    it("skips a junk withdrawal-request id and still advances", () => {
      setMockClaimStakerResult(false, 1001n, 1000n, Cl.some(Cl.uint(9999)));
      registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, MOCK_SIGNER_MANAGER, STX_START, true);
      mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);

      expect(processRewardClaim(wallet1, wallet1, MOCK_SIGNER_MANAGER).result).toBeOk(Cl.none());
      expect(getPendingWithdrawals()).toBeOk(pendingWithdrawalsPage([]));
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

  it("blocks settle-pending-withdrawals reentry from claim-staker-rewards", () => {
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

describe("reentrancy from settle-accepted-withdrawal", () => {
  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    registerMaliciousSignerManager();
    stakeWithPoxAddr(wallet2, SIGNER_SET_MIN_USTX, 2n, 100n);
    stakeForMalicious(wallet1, SIGNER_SET_MIN_USTX, 4n);
    fundAndClaimSignerRewards(2000n, 1n);
    registerForClaims(wallet2, 3n * FEE_PER_CLAIM, wallet2, SIGNER_MANAGER, STX_START, true);
    registerForClaims(
      wallet1,
      3n * FEE_PER_CLAIM,
      wallet1,
      MALICIOUS_SIGNER_MANAGER,
      STX_START,
      true,
    );
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);

    expect(processRewardClaim(wallet2, wallet2, SIGNER_MANAGER).result).toBeOk(Cl.some(Cl.uint(1)));
    setMaliciousWithdrawalRequest(Cl.some(Cl.uint(1)));
    expect(processRewardClaim(wallet1, wallet1, MALICIOUS_SIGNER_MANAGER).result).toBeOk(
      Cl.some(Cl.uint(1)),
    );
    acceptWithdrawal(1n, 30n);
  });

  it("blocks settle-pending-withdrawals reentry from settle-accepted-withdrawal", () => {
    setMaliciousReenterMode(REENTER_SETTLE, wallet1);
    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "settle-pending-withdrawal",
        [Cl.principal(wallet1), Cl.principal(MALICIOUS_SIGNER_MANAGER), Cl.uint(1)],
        wallet3,
      ).result,
    ).toBeOk(Cl.bool(true));
    expect(getMaliciousLastReenterError()).toBeSome(Cl.uint(ERR_REENTRANT_CALL));
    mineUntilWithdrawalListable();
    expect(getPendingWithdrawals()).toBeOk(
      pendingWithdrawalsPage([withdrawalRow(wallet2, 1n)]),
    );
  });
});

describe("withdrawal-request ids that are not a live sBTC pending request", () => {
  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    registerMockSignerManager();
    stakeWithPoxAddr(wallet1, SIGNER_SET_MIN_USTX, 2n, 100n);
    stakeForMock(wallet3, SIGNER_SET_MIN_USTX, 4n);
    fundAndClaimSignerRewards(2000n, 1n);
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    registerForClaims(wallet3, 3n * FEE_PER_CLAIM, wallet3, MOCK_SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
  });

  it("stores a live withdrawal id even if another signer-manager created it", () => {
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.some(Cl.uint(1)));
    setMockClaimStakerResult(false, 1001n, 1000n, Cl.some(Cl.uint(1)));
    expect(processRewardClaim(wallet3, wallet3, MOCK_SIGNER_MANAGER).result).toBeOk(
      Cl.some(Cl.uint(1)),
    );
    acceptWithdrawal(1n, 30n);
    mineUntilWithdrawalListable();
    expect(getPendingWithdrawals()).toBeOk(
      pendingWithdrawalsPage([
        withdrawalRow(wallet1, 1n),
        withdrawalRow(wallet3, 1n, MOCK_SIGNER_MANAGER),
      ]),
    );
    expect(getRegistration(wallet3, MOCK_SIGNER_MANAGER)).toBeSome(
      stxRegistration(2n, STX_FIRST_CLAIM_DIST + 2n),
    );
  });

  it("does not store an already-accepted withdrawal id", () => {
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.some(Cl.uint(1)));
    acceptWithdrawal(1n, 30n);
    setMockClaimStakerResult(false, 1001n, 1000n, Cl.some(Cl.uint(1)));
    expect(processRewardClaim(wallet3, wallet3, MOCK_SIGNER_MANAGER).result).toBeOk(Cl.none());
    mineUntilWithdrawalListable();
    expect(getPendingWithdrawals()).toBeOk(
      pendingWithdrawalsPage([withdrawalRow(wallet1, 1n)]),
    );
  });
});

describe("batch settle SM error does not stick the reentrancy lock", () => {
  beforeEach(() => {
    initPox5();
    registerSignerManager(SIGNER_PRIVATE_KEY);
    stakeWithPoxAddr(wallet1, SIGNER_SET_MIN_USTX, 2n, 100n);
    fundAndClaimSignerRewards(2000n, 1n);
    registerForClaims(wallet1, 3n * FEE_PER_CLAIM, wallet1, SIGNER_MANAGER, STX_START, true);
    mineUntilPastDistribution(STX_FIRST_CLAIM_DIST);
  });

  it("batch settle-pending-withdrawals still allows later gated calls after the SM errors", () => {
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeOk(Cl.some(Cl.uint(1)));
    acceptWithdrawal(1n, 30n);
    expect(settleAcceptedWithdrawalOnSignerManager(1n, wallet2).result).toBeOk(Cl.bool(true));

    expect(
      simnet.callPublicFn(
        "reward-claim-registry",
        "settle-pending-withdrawals",
        [
          Cl.principal(SIGNER_MANAGER),
          Cl.list([
            Cl.tuple({ staker: Cl.principal(wallet1), "request-id": Cl.uint(1) }),
          ]),
        ],
        wallet2,
      ).result,
    ).toBeOk(Cl.uint(1));
    expect(getPendingWithdrawals()).toBeOk(pendingWithdrawalsPage([]));

    // Lock was cleared: the SM error was swallowed and the id pruned.
    expect(processRewardClaim(wallet1, wallet1, SIGNER_MANAGER).result).toBeErr(
      Cl.uint(ERR_ALREADY_CLAIMED),
    );
    expect(settlePendingWithdrawal(wallet1, 1n, wallet2).result).toBeErr(
      Cl.uint(ERR_UNKNOWN_PENDING_WITHDRAWAL),
    );
  });
});
