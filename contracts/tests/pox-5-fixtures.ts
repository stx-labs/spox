import {
  Cl,
  compressPublicKey,
  privateKeyToPublic,
  serializeCVBytes,
  signStructuredData,
  type OptionalCV,
  type UIntCV,
} from "@stacks/transactions";

/**
 * Fixtures for driving the REAL pox-5 / signer-manager / sBTC contracts in
 * simnet, so the reward-claim-registry can be tested against them end to end.
 *
 * Dependency-free beyond @stacks/transactions: key derivation, the SIP-018
 * signer-key signing (hash + secp256k1), and CV (de)serialization all come from
 * it, so there is no need for @noble/* / @scure/* or Node's crypto/Buffer.
 */

// clarinet-sdk applies STX locking only to the boot pox-5, and both
// signer-manager.clar and reward-claim-registry.clar call that principal, so every
// pox-5 interaction here must target it (not a deployer-published copy).
export const POX5 = "ST000000000000000000002AMW42H.pox-5";

const accounts = simnet.getAccounts();
export const deployer = accounts.get("deployer")!;
export const wallet1 = accounts.get("wallet_1")!;
export const wallet2 = accounts.get("wallet_2")!;
export const wallet3 = accounts.get("wallet_3")!;
export const wallet4 = accounts.get("wallet_4")!;

export const SIGNER_MANAGER = `${deployer}.signer-manager`;
export const MOCK_SIGNER_MANAGER = `${deployer}.mock-signer-manager`;
export const MALICIOUS_SIGNER_MANAGER = `${deployer}.malicious-signer-manager`;
export const SWEEP_REGISTRY = `${deployer}.reward-claim-registry`;
export const SBTC_TOKEN =
  "SM3VDXK3WZZSA84XXFKAFAF15NNZX32CTSG82JFQ4.sbtc-token";
export const SBTC_REGISTRY =
  "SM3VDXK3WZZSA84XXFKAFAF15NNZX32CTSG82JFQ4.sbtc-registry";

// Clarinet ships pox-5 with bond/pause admin as the mainnet boot principal
// SP000…002Q6VF78, then rewrites that to the simnet/testnet boot address
// ST000…002AMW42H when loading the contract. initPox5 must call as that
// remapped principal to take over the admin roles.
const POX5_BOOTSTRAP_ADMIN = "ST000000000000000000002AMW42H";

// pox-5 constants (contracts/pox-5.clar)
export const REWARD_CYCLE_LENGTH = 100n;
export const PREPARE_CYCLE_LENGTH = 10n;
/** calculate-rewards runs once per distribution cycle = half a reward cycle. */
export const HALF_CYCLE_LENGTH = REWARD_CYCLE_LENGTH / 2n;
export const SIGNER_SET_MIN_USTX = 50_000_000_000n; // 50k STX
export const RESERVE_RATIO = 1500n; // basis points
export const BASIS_POINTS = 10_000n;
/** pox-5's SIP-018 signing domain. */
const POX5_SIGNER_DOMAIN = { name: "pox-5-signer", version: "1.0.0" };
/** simnet runs with the testnet chain-id. */
const CHAIN_ID = 2147483648;

// The signer-manager's signer key. Any valid secp256k1 private key works.
export const SIGNER_PRIVATE_KEY = "a".repeat(63) + "1";
/** Distinct key for mock-signer-manager so both can be registered in one simnet. */
export const MOCK_SIGNER_PRIVATE_KEY = "b".repeat(63) + "1";
/** Distinct key for malicious-signer-manager reentrancy tests. */
export const MALICIOUS_SIGNER_PRIVATE_KEY = "c".repeat(63) + "1";

/** Reentry modes for malicious-signer-manager.set-reenter-mode. */
export const REENTER_NONE = 0n;
export const REENTER_PROCESS_CLAIMS = 2n;
export const REENTER_CANCEL = 3n;
export const REENTER_SETTLE = 4n;
export const REENTER_ADD_CLAIMS = 5n;
export const REENTER_REGISTER = 6n;

// reward-claim-registry error codes
export const ERR_NOT_REGISTERED = 600n;
export const ERR_MAX_NUM_CLAIMS_EXCEEDED = 601n;
export const ERR_NOT_ADMIN = 602n;
export const ERR_NO_CURRENT_POSITION = 603n;
export const ERR_ZERO_NUM_CLAIMS = 604n;
export const ERR_ALREADY_CLAIMED = 605n;
export const ERR_REWARDS_NOT_CALCULATED = 606n;
export const ERR_UNKNOWN_PENDING_WITHDRAWAL = 607n;
export const ERR_INVALID_START_REWARD_CYCLE = 608n;
export const ERR_UNAUTHORIZED = 609n;
export const ERR_ALREADY_REGISTERED = 610n;
export const ERR_SIGNER_MANAGER_MISMATCH = 611n;
export const ERR_REENTRANT_CALL = 612n;

/** reward-claim-registry's default fee-per-claim. */
export const FEE_PER_CLAIM = 10_000n;

// ---------------------------------------------------------------------------
// low-level helpers
// ---------------------------------------------------------------------------

export function stxToUStx(stx: number): bigint {
  return BigInt(stx) * 1_000_000n;
}

/** Rewards left after pox-5 skims its reserve cut. */
export function stxRewards(rewards: bigint): bigint {
  return rewards - (rewards * RESERVE_RATIO) / BASIS_POINTS;
}

export function burnHeight(): bigint {
  return BigInt(simnet.burnBlockHeight);
}

/** Mine burn blocks until `target` is reached (no-op if already past it). */
export function mineUntil(target: bigint) {
  const current = burnHeight();
  if (target > current) {
    simnet.mineEmptyBurnBlocks(Number(target - current));
  }
}

/** Bitcoin blocks after indexing before get-pending-withdrawals consults sBTC. */
export const WITHDRAWAL_MIN_BURN_AGE = 7n;

/** Mine enough burn blocks for an indexed withdrawal to pass the listing age gate. */
export function mineUntilWithdrawalListable() {
  simnet.mineEmptyBurnBlocks(Number(WITHDRAWAL_MIN_BURN_AGE));
}

export function rewardCycleToBurnHeight(cycle: bigint): bigint {
  const { result } = simnet.callReadOnlyFn(
    POX5,
    "reward-cycle-to-burn-height",
    [Cl.uint(cycle)],
    deployer,
  );
  return (result as unknown as { value: bigint }).value;
}

export function currentRewardCycle(): bigint {
  const { result } = simnet.callReadOnlyFn(
    POX5,
    "current-pox-reward-cycle",
    [],
    deployer,
  );
  return (result as unknown as { value: bigint }).value;
}

export function currentDistributionCycle(): bigint {
  const { result } = simnet.callReadOnlyFn(
    POX5,
    "current-distribution-cycle",
    [],
    deployer,
  );
  return (result as unknown as { value: bigint }).value;
}

export function distributionCycleToBurnHeight(cycle: bigint): bigint {
  const { result } = simnet.callReadOnlyFn(
    POX5,
    "distribution-cycle-to-burn-height",
    [Cl.uint(cycle)],
    deployer,
  );
  return (result as unknown as { value: bigint }).value;
}

/**
 * Mine until current-distribution-cycle > distCycle, so a registration whose
 * next-claim-distribution is `distCycle` becomes time-eligible (k < CD), then
 * run pox-5 calculate-rewards so that claim distribution is covered
 * (compute at CD = distCycle+1). Pass active bond indexes when bonds are
 * live; otherwise pox-5 rejects an empty bond-periods list.
 */
export function mineUntilPastDistribution(
  distCycle: bigint,
  bondPeriods: bigint[] = [],
) {
  mineUntil(distributionCycleToBurnHeight(distCycle + 1n));
  return calculateRewards(bondPeriods);
}

/** Run pox-5 calculate-rewards for the current distribution cycle. */
export function calculateRewards(bondPeriods: bigint[] = []) {
  return simnet.callPublicFn(
    POX5,
    "calculate-rewards",
    [Cl.list(bondPeriods.map((b) => Cl.uint(b)))],
    deployer,
  );
}

/**
 * Initial next-claim-distribution for start reward-cycle `s`:
 * oneClaimPerRewardCycle -> 2s+1 (step 2); else -> 2s (step 1).
 */
export function initialNextClaimDistribution(
  startRewardCycle: bigint,
  oneClaimPerRewardCycle: boolean,
): bigint {
  const step = oneClaimPerRewardCycle ? 2n : 1n;
  return 2n * startRewardCycle + step - 1n;
}

export function sbtcBalance(who: string): bigint {
  const { result } = simnet.callReadOnlyFn(
    SBTC_TOKEN,
    "get-balance",
    [Cl.principal(who)],
    deployer,
  );
  // (ok uint)
  const ok = (result as unknown as { value: { value: bigint } }).value;
  return ok.value;
}

export function stxBalance(who: string): bigint {
  return simnet.getAssetsMap().get("STX")!.get(who)!;
}

// ---------------------------------------------------------------------------
// pox-5 setup
// ---------------------------------------------------------------------------

/**
 * Initialize the boot pox-5: set short burnchain parameters so cycles are cheap
 * to advance, and move both admin roles from the baked-in bootstrap principal to
 * the deployer.
 */
export function initPox5() {
  simnet.callPublicFn(
    POX5,
    "set-burnchain-parameters",
    [
      Cl.uint(0),
      Cl.uint(PREPARE_CYCLE_LENGTH),
      Cl.uint(REWARD_CYCLE_LENGTH),
      Cl.uint(1),
    ],
    deployer,
  );
  simnet.callPublicFn(
    POX5,
    "set-bond-admin",
    [Cl.principal(deployer)],
    POX5_BOOTSTRAP_ADMIN,
  );
  simnet.callPublicFn(
    POX5,
    "set-pause-admin",
    [Cl.principal(deployer)],
    POX5_BOOTSTRAP_ADMIN,
  );
}

let authIdCounter = 1000n;

/**
 * Sign a pox-5 signer-key grant for `signerManager` (SIP-018 structured data,
 * converted from VRS to the RSV layout pox-5 expects).
 */
function signSignerKeyGrant(
  signerManager: string,
  authId: bigint,
  privateKey: string,
): string {
  const message = Cl.tuple({
    "signer-manager": Cl.principal(signerManager),
    topic: Cl.stringAscii("grant-authorization"),
    "auth-id": Cl.uint(authId),
  });
  const domain = Cl.tuple({
    name: Cl.stringAscii(POX5_SIGNER_DOMAIN.name),
    version: Cl.stringAscii(POX5_SIGNER_DOMAIN.version),
    "chain-id": Cl.uint(CHAIN_ID),
  });
  // encode -> sha256 -> secp256k1 sign in RSV order, exactly pox-5's format
  return signStructuredData({ message, domain, privateKey });
}

/**
 * Grant a signer key to the real signer-manager and register it with pox-5.
 * `register-self` performs both steps and is admin-only, so it runs as deployer
 * (the signer-manager's default admin).
 */
export function registerSignerManager(privateKey: string) {
  const authId = authIdCounter++;
  const signerKey = compressPublicKey(privateKeyToPublic(privateKey));
  const signerSig = signSignerKeyGrant(SIGNER_MANAGER, authId, privateKey);
  return simnet.callPublicFn(
    "signer-manager",
    "register-self",
    [
      Cl.principal(SIGNER_MANAGER),
      Cl.bufferFromHex(signerKey),
      Cl.uint(authId),
      Cl.bufferFromHex(signerSig),
    ],
    deployer,
  );
}

/**
 * Register mock-signer-manager with pox-5 so tests can stake under it and drive
 * claim-rewards / claim-staker-rewards error injection.
 */
export function registerMockSignerManager(privateKey = MOCK_SIGNER_PRIVATE_KEY) {
  const authId = authIdCounter++;
  const signerKey = compressPublicKey(privateKeyToPublic(privateKey));
  const signerSig = signSignerKeyGrant(MOCK_SIGNER_MANAGER, authId, privateKey);
  return simnet.callPublicFn(
    "mock-signer-manager",
    "register-self",
    [
      Cl.principal(MOCK_SIGNER_MANAGER),
      Cl.bufferFromHex(signerKey),
      Cl.uint(authId),
      Cl.bufferFromHex(signerSig),
    ],
    deployer,
  );
}

/** Stake `amount` uSTX under mock-signer-manager. */
export function stakeForMock(staker: string, amount: bigint, numCycles: bigint) {
  return simnet.callPublicFn(
    POX5,
    "stake",
    [
      Cl.principal(MOCK_SIGNER_MANAGER),
      Cl.uint(amount),
      Cl.uint(numCycles),
      Cl.uint(burnHeight()),
      Cl.none(),
    ],
    staker,
  );
}

export function setMockClaimRewardsResult(shouldError: boolean, code = 1001n) {
  return simnet.callPublicFn(
    "mock-signer-manager",
    "set-claim-rewards-result",
    [Cl.bool(shouldError), Cl.uint(code)],
    deployer,
  );
}

export function registerMaliciousSignerManager(
  privateKey = MALICIOUS_SIGNER_PRIVATE_KEY,
) {
  const authId = authIdCounter++;
  const signerKey = compressPublicKey(privateKeyToPublic(privateKey));
  const signerSig = signSignerKeyGrant(MALICIOUS_SIGNER_MANAGER, authId, privateKey);
  return simnet.callPublicFn(
    "malicious-signer-manager",
    "register-self",
    [
      Cl.principal(MALICIOUS_SIGNER_MANAGER),
      Cl.bufferFromHex(signerKey),
      Cl.uint(authId),
      Cl.bufferFromHex(signerSig),
    ],
    deployer,
  );
}

/** Stake `amount` uSTX under malicious-signer-manager. */
export function stakeForMalicious(staker: string, amount: bigint, numCycles: bigint) {
  return simnet.callPublicFn(
    POX5,
    "stake",
    [
      Cl.principal(MALICIOUS_SIGNER_MANAGER),
      Cl.uint(amount),
      Cl.uint(numCycles),
      Cl.uint(burnHeight()),
      Cl.none(),
    ],
    staker,
  );
}

export function setMaliciousReenterMode(mode: bigint, staker: string) {
  return simnet.callPublicFn(
    "malicious-signer-manager",
    "set-reenter-mode",
    [Cl.uint(mode), Cl.principal(staker)],
    deployer,
  );
}

export function setMaliciousWithdrawalRequest(wid: OptionalCV<UIntCV>) {
  return simnet.callPublicFn(
    "malicious-signer-manager",
    "set-withdrawal-request",
    [wid],
    deployer,
  );
}

export function getMaliciousLastReenterError() {
  return simnet.callReadOnlyFn(
    "malicious-signer-manager",
    "get-last-reenter-error",
    [],
    deployer,
  ).result;
}

export function setMockSettleResult(shouldError: boolean, code = 1001n) {
  return simnet.callPublicFn(
    "mock-signer-manager",
    "set-settle-result",
    [Cl.bool(shouldError), Cl.uint(code)],
    deployer,
  );
}

export function setMockClaimStakerResult(
  shouldError: boolean,
  code = 1001n,
  earned = 1000n,
  wid: OptionalCV<UIntCV> = Cl.none(),
) {
  return simnet.callPublicFn(
    "mock-signer-manager",
    "set-claim-staker-result",
    [Cl.bool(shouldError), Cl.uint(code), Cl.uint(earned), wid],
    deployer,
  );
}

/** A valid pox-addr (p2pkh, version 0x00, 20-byte hash) for the L1 path. */
export const POX_ADDR = {
  version: Uint8Array.from([0x00]),
  hashbytes: new Uint8Array(20).fill(0x11),
};

/**
 * Encode pox-addr calldata the way signer-manager's validate-stake! expects:
 * a consensus-serialized `{ pox-addr: { version, hashbytes }, max-fee }`.
 */
export function poxAddrCalldata(maxFee: bigint) {
  const cv = Cl.tuple({
    "pox-addr": Cl.tuple({
      version: Cl.buffer(POX_ADDR.version),
      hashbytes: Cl.buffer(POX_ADDR.hashbytes),
    }),
    "max-fee": Cl.uint(maxFee),
  });
  return Cl.some(Cl.buffer(serializeCVBytes(cv)));
}

// --- protocol bonds (sBTC-collateralized path, no Bitcoin lockup proof) ---

/** stx-value-ratio used by the bond fixtures: 1 uSTX = 100 sat. */
export const BOND_STX_VALUE_RATIO = 10_000_000n;
/** min-ustx-ratio used by the bond fixtures: 10%. */
export const BOND_MIN_USTX_RATIO = 1_000n;

/** pox-5's min uSTX required to bond `sats` at the fixture ratios. */
export function minUstxForSats(sats: bigint): bigint {
  const { result } = simnet.callReadOnlyFn(
    POX5,
    "min-ustx-for-sats-amount",
    [Cl.uint(sats), Cl.uint(BOND_STX_VALUE_RATIO), Cl.uint(BOND_MIN_USTX_RATIO)],
    deployer,
  );
  return (result as unknown as { value: bigint }).value;
}

export function bondPeriodToRewardCycle(bondIndex: bigint): bigint {
  const { result } = simnet.callReadOnlyFn(
    POX5,
    "bond-period-to-reward-cycle",
    [Cl.uint(bondIndex)],
    deployer,
  );
  return (result as unknown as { value: bigint }).value;
}

/**
 * Set up protocol bond `bondIndex` (bond-admin = deployer after initPox5) with
 * an allowlist covering `stakers`. Cribbed from core-contract-tests'
 * "setting up and starting a bond" scenario.
 */
export function setupBond(bondIndex: bigint, stakers: string[], maxSats: bigint) {
  return simnet.callPublicFn(
    POX5,
    "setup-bond",
    [
      Cl.uint(bondIndex),
      Cl.uint(300), // target-rate (bps)
      Cl.uint(BOND_STX_VALUE_RATIO),
      Cl.uint(BOND_MIN_USTX_RATIO),
      Cl.buffer(new Uint8Array()), // early-unlock-bytes (unused on the sBTC path)
      Cl.list(
        stakers.map((s) =>
          Cl.tuple({ staker: Cl.principal(s), "max-sats": Cl.uint(maxSats) }),
        ),
      ),
    ],
    deployer,
  );
}

/**
 * Register `staker` for `bondIndex` under the real signer-manager, collateralized
 * with `sats` sBTC (btc-lockup = (err sats), the non-L1 path). Uses the minimum
 * qualifying uSTX so the position is valid.
 */
export function registerForBond(
  staker: string,
  bondIndex: bigint,
  sats: bigint,
) {
  return simnet.callPublicFn(
    POX5,
    "register-for-bond",
    [
      Cl.uint(bondIndex),
      Cl.principal(SIGNER_MANAGER),
      Cl.uint(minUstxForSats(sats)),
      Cl.error(Cl.uint(sats)), // (err sats) -> sBTC-collateralized bond
      Cl.none(),
    ],
    staker,
  );
}

/** Stake `amount` uSTX under the real signer-manager WITH an L1 pox-addr. */
export function stakeWithPoxAddr(
  staker: string,
  amount: bigint,
  numCycles: bigint,
  maxFee: bigint,
) {
  return simnet.callPublicFn(
    POX5,
    "stake",
    [
      Cl.principal(SIGNER_MANAGER),
      Cl.uint(amount),
      Cl.uint(numCycles),
      Cl.uint(burnHeight()),
      poxAddrCalldata(maxFee),
    ],
    staker,
  );
}

/** Stake `amount` uSTX under the real signer-manager, as `staker`. */
export function stakeFor(staker: string, amount: bigint, numCycles: bigint) {
  return simnet.callPublicFn(
    POX5,
    "stake",
    [
      Cl.principal(SIGNER_MANAGER),
      Cl.uint(amount),
      Cl.uint(numCycles),
      Cl.uint(burnHeight()),
      Cl.none(),
    ],
    staker,
  );
}

/** Switch a live stake to a different signer-manager via pox-5 `stake-update`. */
export function stakeUpdate(
  staker: string,
  newSignerManager: string,
  oldSignerManager: string,
  cyclesToExtend = 0n,
  amountIncrease = 0n,
) {
  return simnet.callPublicFn(
    POX5,
    "stake-update",
    [
      Cl.principal(newSignerManager),
      Cl.principal(oldSignerManager),
      Cl.uint(cyclesToExtend),
      Cl.uint(amountIncrease),
      Cl.none(),
    ],
    staker,
  );
}

/**
 * Fund pox-5 with sBTC for `rewardCycle`, advance to that cycle's distribution
 * boundary, and run pox-5 calculate-rewards -- but NOT the signer-manager
 * claim-rewards step. This leaves pox-5's get-earned > 0 for the signer, which is
 * the state process-reward-claim now heals itself (it calls claim-rewards).
 */
export function fundAndCalculateRewards(rewards: bigint, rewardCycle: bigint) {
  simnet.callPublicFn(
    SBTC_TOKEN,
    "transfer",
    [Cl.uint(rewards), Cl.principal(deployer), Cl.principal(POX5), Cl.none()],
    deployer,
  );
  mineUntil(rewardCycleToBurnHeight(rewardCycle) + HALF_CYCLE_LENGTH);
  return calculateRewards();
}

/**
 * Move rewards into the signer-manager for `rewardCycle`: fund + calculate (see
 * fundAndCalculateRewards), then the signer-manager claim-rewards. This is the
 * fully pre-pulled state (get-earned == 0); process-reward-claim then pays
 * directly without needing to pull.
 */
export function fundAndClaimSignerRewards(rewards: bigint, rewardCycle: bigint) {
  fundAndCalculateRewards(rewards, rewardCycle);
  return simnet.callPublicFn(
    "signer-manager",
    "claim-rewards",
    [Cl.list([]), Cl.uint(rewardCycle)],
    deployer,
  );
}

/** pox-5's get-earned for a signer/cycle/scope: rewards still owed (unpulled). */
export function getEarned(
  signerManager: string,
  rewardCycle: bigint,
  bondIndex: OptionalCV<UIntCV>,
): bigint {
  const { result } = simnet.callReadOnlyFn(
    POX5,
    "get-earned",
    [Cl.principal(signerManager), Cl.uint(rewardCycle), bondIndex],
    deployer,
  );
  return (result as unknown as { value: bigint }).value;
}

// ---------------------------------------------------------------------------
// reward-claim-registry helpers
// ---------------------------------------------------------------------------

export function registerForClaims(
  staker: string,
  numClaims: bigint,
  sender: string,
  signerManager: string,
  startRewardCycle: bigint,
  oneClaimPerRewardCycle: boolean,
) {
  return simnet.callPublicFn(
    "reward-claim-registry",
    "register-for-claims",
    [
      Cl.principal(staker),
      Cl.principal(signerManager),
      Cl.uint(startRewardCycle),
      Cl.bool(oneClaimPerRewardCycle),
      Cl.uint(numClaims),
    ],
    sender,
  );
}

export type RegisterManyEntry = {
  staker: string;
  startRewardCycle: bigint;
  oneClaimPerRewardCycle: boolean;
  numClaims: bigint;
};

export function registerManyForClaims(
  stakers: RegisterManyEntry[],
  sender: string,
  signerManager: string,
) {
  return simnet.callPublicFn(
    "reward-claim-registry",
    "register-many-for-claims",
    [
      Cl.principal(signerManager),
      Cl.list(
        stakers.map((entry) =>
          Cl.tuple({
            staker: Cl.principal(entry.staker),
            "start-reward-cycle": Cl.uint(entry.startRewardCycle),
            "one-claim-per-reward-cycle": Cl.bool(entry.oneClaimPerRewardCycle),
            "num-claims": Cl.uint(entry.numClaims),
          }),
        ),
      ),
    ],
    sender,
  );
}

export function addClaims(
  staker: string,
  numClaims: bigint,
  sender: string,
  signerManager: string,
) {
  return simnet.callPublicFn(
    "reward-claim-registry",
    "add-claims",
    [Cl.principal(staker), Cl.principal(signerManager), Cl.uint(numClaims)],
    sender,
  );
}

export function cancelRegistration(
  staker: string,
  sender: string,
  signerManager: string,
) {
  return simnet.callPublicFn(
    "reward-claim-registry",
    "cancel-registration",
    [Cl.principal(staker), Cl.principal(signerManager)],
    sender,
  );
}

export function cancelManyRegistrations(
  stakers: string[],
  sender: string,
  signerManager: string,
) {
  return simnet.callPublicFn(
    "reward-claim-registry",
    "cancel-many-registrations",
    [
      Cl.principal(signerManager),
      Cl.list(stakers.map((staker) => Cl.principal(staker))),
    ],
    sender,
  );
}

export function processRewardClaim(
  staker: string,
  sender: string,
  signerManager: string,
) {
  return simnet.callPublicFn(
    "reward-claim-registry",
    "process-reward-claim",
    [Cl.principal(staker), Cl.principal(signerManager)],
    sender,
  );
}

export function getRegistration(staker: string, signerManager: string) {
  return simnet.callReadOnlyFn(
    "reward-claim-registry",
    "get-registration",
    [Cl.principal(staker), Cl.principal(signerManager)],
    deployer,
  ).result;
}

export function getMaxProcessedDistribution(staker: string) {
  return simnet.callReadOnlyFn(
    "reward-claim-registry",
    "get-max-processed-distribution",
    [Cl.principal(staker)],
    deployer,
  ).result;
}

export function getPendingClaims(cursor: OptionalCV = Cl.none()) {
  return simnet.callReadOnlyFn(
    "reward-claim-registry",
    "get-pending-claims",
    [cursor],
    deployer,
  ).result;
}

export function getRegistrations(cursor: OptionalCV = Cl.none()) {
  return simnet.callReadOnlyFn(
    "reward-claim-registry",
    "get-registrations",
    [cursor],
    deployer,
  ).result;
}

export function getPendingWithdrawals(cursor: OptionalCV = Cl.none()) {
  return simnet.callReadOnlyFn(
    "reward-claim-registry",
    "get-pending-withdrawals",
    [cursor],
    deployer,
  ).result;
}

export function getWithdrawals(cursor: OptionalCV = Cl.none()) {
  return simnet.callReadOnlyFn(
    "reward-claim-registry",
    "get-withdrawals",
    [cursor],
    deployer,
  ).result;
}

// The sBTC registry's current-signer-principal (allowed to accept/reject
// withdrawals) defaults to the sBTC deployer.
export const SBTC_SIGNER = "SM3VDXK3WZZSA84XXFKAFAF15NNZX32CTSG82JFQ4";
const SBTC_WITHDRAWAL =
  "SM3VDXK3WZZSA84XXFKAFAF15NNZX32CTSG82JFQ4.sbtc-withdrawal";

/** The burn header hash at `height` (accept-withdrawal-request's fork check). */
function burnHeaderHex(height: bigint): string {
  const r = simnet.execute(`(get-burn-block-info? header-hash u${height})`);
  // (some (buff 32)) -> the 32-byte header hash as hex
  return (r.result as unknown as { value: { value: string } }).value.value;
}

/** sBTC signers REJECT withdrawal `requestId` -> status becomes (some false). */
export function rejectWithdrawal(requestId: bigint) {
  return simnet.callPublicFn(
    SBTC_WITHDRAWAL,
    "reject-withdrawal-request",
    [Cl.uint(requestId), Cl.uint(0)],
    SBTC_SIGNER,
  );
}

/** Direct signer-manager settle; deletes its map entry without touching the registry. */
export function settleAcceptedWithdrawalOnSignerManager(requestId: bigint, sender: string) {
  return simnet.callPublicFn(
    "signer-manager",
    "settle-accepted-withdrawal",
    [Cl.uint(requestId)],
    sender,
  );
}

/** sBTC signers ACCEPT withdrawal `requestId` -> status becomes (some true). */
export function acceptWithdrawal(requestId: bigint, fee: bigint) {
  const height = BigInt(simnet.burnBlockHeight - 1);
  return simnet.callPublicFn(
    SBTC_WITHDRAWAL,
    "accept-withdrawal-request",
    [
      Cl.uint(requestId),
      Cl.buffer(new Uint8Array(32)),
      Cl.uint(0),
      Cl.uint(0),
      Cl.uint(fee),
      Cl.bufferFromHex(burnHeaderHex(height)),
      Cl.uint(height),
      Cl.buffer(new Uint8Array(32)),
    ],
    SBTC_SIGNER,
  );
}

/**
 * Full happy-path setup: init pox-5, register the signer-manager, stake
 * `stakers`, then pull `rewards` sBTC through to the signer-manager so each
 * staker has something claimable for reward cycle 1.
 */
export function setupClaimableStakers(stakers: string[], rewards: bigint) {
  initPox5();
  registerSignerManager(SIGNER_PRIVATE_KEY);
  for (const staker of stakers) {
    stakeFor(staker, SIGNER_SET_MIN_USTX, 2n);
  }
  fundAndClaimSignerRewards(rewards, 1n);
}
