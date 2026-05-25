import QEDGen.Solana.Account
import QEDGen.Solana.Cpi
import QEDGen.Solana.State
import QEDGen.Solana.Valid

namespace Lending

open QEDGen.Solana

inductive Status where
  | Uninitialized : Status
  | Active : Status
  | Paused : Status
  deriving DecidableEq, Repr

structure State where
  authority : Pubkey
  total_deposits : Nat
  total_borrows : Nat
  interest_rate : Nat
  status : Status
  deriving DecidableEq, Repr

def init_poolTransition (s : State) (signer : Pubkey) (rate : Nat) : Option State :=
  let authority := signer
  if rate > 0 then
    some { s with interest_rate := rate, total_deposits := 0, total_borrows := 0, status := .Active }
  else
    none

def depositTransition (s : State) (signer : Pubkey) (amount : Nat) : Option State :=
  if amount > 0 then
    some { s with total_deposits := s.total_deposits + amount, status := .Active }
  else
    none

def borrowTransition (s : State) (signer : Pubkey) (amount : Nat) (collateral : Nat) : Option State :=
  let borrower := signer
  if amount > 0 ∧ collateral > 0 then
    some { s with amount := amount, collateral := collateral, status := .Active }
  else
    none

def repayTransition (s : State) (signer : Pubkey) : Option State :=
  let borrower := signer
  some { s with amount := 0, collateral := 0, status := .Empty }

def liquidateTransition (s : State) (signer : Pubkey) : Option State :=
  if s.amount > s.collateral then
    some { s with amount := 0, status := .Liquidated }
  else
    none

/-- deposit transfer envelope: depositor_ta → pool_vault amount amount authority depositor.
    Verifies CPI shape (program ID, account list, discriminator).
    Amount serialization and SPL Token execution are SDK/runtime
    trust per VERIFICATION_SCOPE.md. -/
def build_deposit_transfer (from_pk to_pk authority_pk : Pubkey) : CpiInstruction :=
  { programId := TOKEN_PROGRAM_ID
  , accounts :=
      [ ⟨from_pk, false, true⟩
      , ⟨to_pk, false, true⟩
      , ⟨authority_pk, true, false⟩
      ]
  , data := DISC_TRANSFER }

theorem deposit_transfer_correct (from_pk to_pk authority_pk : Pubkey) :
    let cpi := build_deposit_transfer from_pk to_pk authority_pk
    targetsProgram cpi TOKEN_PROGRAM_ID ∧
    accountAt cpi 0 from_pk false true ∧
    accountAt cpi 1 to_pk false true ∧
    accountAt cpi 2 authority_pk true false ∧
    hasDiscriminator cpi DISC_TRANSFER := by
  unfold build_deposit_transfer targetsProgram accountAt hasDiscriminator
  exact ⟨rfl, rfl, rfl, rfl, rfl⟩

/-- borrow transfer envelope: pool_vault → borrower_ta amount amount authority pool.
    Verifies CPI shape (program ID, account list, discriminator).
    Amount serialization and SPL Token execution are SDK/runtime
    trust per VERIFICATION_SCOPE.md. -/
def build_borrow_transfer (from_pk to_pk authority_pk : Pubkey) : CpiInstruction :=
  { programId := TOKEN_PROGRAM_ID
  , accounts :=
      [ ⟨from_pk, false, true⟩
      , ⟨to_pk, false, true⟩
      , ⟨authority_pk, true, false⟩
      ]
  , data := DISC_TRANSFER }

theorem borrow_transfer_correct (from_pk to_pk authority_pk : Pubkey) :
    let cpi := build_borrow_transfer from_pk to_pk authority_pk
    targetsProgram cpi TOKEN_PROGRAM_ID ∧
    accountAt cpi 0 from_pk false true ∧
    accountAt cpi 1 to_pk false true ∧
    accountAt cpi 2 authority_pk true false ∧
    hasDiscriminator cpi DISC_TRANSFER := by
  unfold build_borrow_transfer targetsProgram accountAt hasDiscriminator
  exact ⟨rfl, rfl, rfl, rfl, rfl⟩

/-- repay transfer envelope: borrower_ta → pool_vault amount amount authority borrower.
    Verifies CPI shape (program ID, account list, discriminator).
    Amount serialization and SPL Token execution are SDK/runtime
    trust per VERIFICATION_SCOPE.md. -/
def build_repay_transfer (from_pk to_pk authority_pk : Pubkey) : CpiInstruction :=
  { programId := TOKEN_PROGRAM_ID
  , accounts :=
      [ ⟨from_pk, false, true⟩
      , ⟨to_pk, false, true⟩
      , ⟨authority_pk, true, false⟩
      ]
  , data := DISC_TRANSFER }

theorem repay_transfer_correct (from_pk to_pk authority_pk : Pubkey) :
    let cpi := build_repay_transfer from_pk to_pk authority_pk
    targetsProgram cpi TOKEN_PROGRAM_ID ∧
    accountAt cpi 0 from_pk false true ∧
    accountAt cpi 1 to_pk false true ∧
    accountAt cpi 2 authority_pk true false ∧
    hasDiscriminator cpi DISC_TRANSFER := by
  unfold build_repay_transfer targetsProgram accountAt hasDiscriminator
  exact ⟨rfl, rfl, rfl, rfl, rfl⟩

/-- liquidate transfer envelope: pool_vault → liquidator_ta amount amount authority pool.
    Verifies CPI shape (program ID, account list, discriminator).
    Amount serialization and SPL Token execution are SDK/runtime
    trust per VERIFICATION_SCOPE.md. -/
def build_liquidate_transfer (from_pk to_pk authority_pk : Pubkey) : CpiInstruction :=
  { programId := TOKEN_PROGRAM_ID
  , accounts :=
      [ ⟨from_pk, false, true⟩
      , ⟨to_pk, false, true⟩
      , ⟨authority_pk, true, false⟩
      ]
  , data := DISC_TRANSFER }

theorem liquidate_transfer_correct (from_pk to_pk authority_pk : Pubkey) :
    let cpi := build_liquidate_transfer from_pk to_pk authority_pk
    targetsProgram cpi TOKEN_PROGRAM_ID ∧
    accountAt cpi 0 from_pk false true ∧
    accountAt cpi 1 to_pk false true ∧
    accountAt cpi 2 authority_pk true false ∧
    hasDiscriminator cpi DISC_TRANSFER := by
  unfold build_liquidate_transfer targetsProgram accountAt hasDiscriminator
  exact ⟨rfl, rfl, rfl, rfl, rfl⟩

/-- Invariant: collateral_backing -/
theorem collateral_backing (s : State) : ∀ l : Loan.Active, l.collateral > 0 := by sorry

inductive Operation where
  | init_pool (rate : Nat)
  | deposit (amount : Nat)
  | borrow (amount : Nat) (collateral : Nat)
  | repay
  | liquidate
  deriving Repr, DecidableEq, BEq

def applyOp (s : State) (signer : Pubkey) : Operation → Option State
  | .init_pool rate => init_poolTransition s signer rate
  | .deposit amount => depositTransition s signer amount
  | .borrow amount collateral => borrowTransition s signer amount collateral
  | .repay => repayTransition s signer
  | .liquidate => liquidateTransition s signer

def pool_solvency (s : State) : Prop := s.total_deposits ≥ s.total_borrows

theorem pool_solvency_preserved_by_init_pool (s s' : State) (signer : Pubkey) (rate : Nat)
    (h_inv : pool_solvency s) (h : init_poolTransition s signer rate = some s') :
    pool_solvency s' := sorry

theorem pool_solvency_preserved_by_deposit (s s' : State) (signer : Pubkey) (amount : Nat)
    (h_inv : pool_solvency s) (h : depositTransition s signer amount = some s') :
    pool_solvency s' := sorry

theorem pool_solvency_preserved_by_borrow (s s' : State) (signer : Pubkey) (amount : Nat) (collateral : Nat)
    (h_inv : pool_solvency s) (h : borrowTransition s signer amount collateral = some s') :
    pool_solvency s' := sorry

theorem pool_solvency_preserved_by_repay (s s' : State) (signer : Pubkey)
    (h_inv : pool_solvency s) (h : repayTransition s signer = some s') :
    pool_solvency s' := sorry

theorem pool_solvency_preserved_by_liquidate (s s' : State) (signer : Pubkey)
    (h_inv : pool_solvency s) (h : liquidateTransition s signer = some s') :
    pool_solvency s' := sorry

/-- pool_solvency is preserved by every operation. -/
theorem pool_solvency_invariant (s s' : State) (signer : Pubkey) (op : Operation)
    (h_inv : pool_solvency s) (h : applyOp s signer op = some s') :
    pool_solvency s' := sorry

-- ============================================================================
-- Abort conditions — operations must reject under specified conditions
-- ============================================================================

theorem init_pool_aborts_if_InvalidAmount (s : State) (signer : Pubkey) (rate : Nat)
    (h : ¬(rate > 0)) : init_poolTransition s signer rate = none := sorry

theorem deposit_aborts_if_InvalidAmount (s : State) (signer : Pubkey) (amount : Nat)
    (h : ¬(amount > 0)) : depositTransition s signer amount = none := sorry

theorem borrow_aborts_if_InvalidAmount (s : State) (signer : Pubkey) (amount : Nat) (collateral : Nat)
    (h : ¬(amount > 0 ∧ collateral > 0)) : borrowTransition s signer amount collateral = none := sorry

theorem liquidate_aborts_if_AccountHealthy (s : State) (signer : Pubkey)
    (h : ¬(s.amount > s.collateral)) : liquidateTransition s signer = none := sorry

-- ============================================================================
-- Cover properties — reachability (existential proofs)
-- ============================================================================

/-- borrow_repay_cycle — trace [init_pool, deposit, borrow, repay] is reachable. -/
theorem cover_borrow_repay_cycle : ∃ (s0 : State) (signer : Pubkey),
    ∃ (v0_0 : Nat), ∃ (s1 : State), init_poolTransition s0 signer v0_0 = some s1 ∧
      ∃ (v1_0 : Nat), ∃ (s2 : State), depositTransition s1 signer v1_0 = some s2 ∧
        ∃ (v2_0 : Nat) (v2_1 : Nat), ∃ (s3 : State), borrowTransition s2 signer v2_0 v2_1 = some s3 ∧
repayTransition s3 signer ≠ none := by
  let pk : Pubkey := ⟨0, 0, 0, 0⟩
  let s0 : State := ⟨pk, 0, 0, 0, .Uninitialized⟩
  let s1 : State := ⟨pk, 0, 0, 1, .Active⟩
  let s2 : State := ⟨pk, 1, 0, 1, .Active⟩
  let s3 : State := ⟨pk, 1, 0, 1, .Active⟩
  exact ⟨s0, pk, 1, s1, by decide, 1, s2, by decide, 1, 1, s3, by decide, by decide⟩

/-- liquidation_path — trace [init_pool, deposit, borrow, liquidate] is reachable. -/
theorem cover_liquidation_path : ∃ (s0 : State) (signer : Pubkey),
    ∃ (v0_0 : Nat), ∃ (s1 : State), init_poolTransition s0 signer v0_0 = some s1 ∧
      ∃ (v1_0 : Nat), ∃ (s2 : State), depositTransition s1 signer v1_0 = some s2 ∧
        ∃ (v2_0 : Nat) (v2_1 : Nat), ∃ (s3 : State), borrowTransition s2 signer v2_0 v2_1 = some s3 ∧
liquidateTransition s3 signer ≠ none := by
  let pk : Pubkey := ⟨0, 0, 0, 0⟩
  let s0 : State := ⟨pk, 0, 0, 0, .Uninitialized⟩
  let s1 : State := ⟨pk, 0, 0, 1, .Active⟩
  let s2 : State := ⟨pk, 1, 0, 1, .Active⟩
  let s3 : State := ⟨pk, 1, 0, 1, .Active⟩
  exact ⟨s0, pk, 1, s1, by decide, 1, s2, by decide, 1, 1, s3, by decide, by decide⟩

-- ============================================================================
-- Liveness properties — bounded reachability (leads-to)
-- ============================================================================

def applyOps (s : State) (signer : Pubkey) : List Operation → Option State
  | [] => some s
  | op :: ops => match applyOp s signer op with
    | some s' => applyOps s' signer ops
    | none => none

/-- loan_settles — from Active leads to Empty within 1 steps via [repay]. -/
theorem liveness_loan_settles (s : State) (signer : Pubkey)
    (h : s.status = .Active) :
    ∃ ops s', ops.length ≤ 1 ∧ applyOps s signer ops = some s' ∧ s'.status = .Empty := by sorry

-- ============================================================================
-- Environment — properties hold under external state changes
-- ============================================================================

theorem pool_solvency_under_interest_rate_change (s : State) (new_interest_rate : Nat)
    (h_c0 : interest_rate > 0)
    (h_inv : pool_solvency s) :
    pool_solvency { s with interest_rate := new_interest_rate } := by
  unfold pool_solvency at h_inv ⊢; dsimp; exact h_inv

-- ============================================================================
-- Overflow safety obligations (auto-generated for operations with add effects)
-- ============================================================================

theorem deposit_overflow_safe (s s' : State) (signer : Pubkey) (amount : Nat)
    (h_valid : valid_u64 s.total_deposits ∧ valid_u64 s.total_borrows ∧ valid_u64 s.interest_rate)
    (h_inv_pool_solvency : pool_solvency s)
    (h : depositTransition s signer amount = some s') :
    valid_u64 s'.total_deposits ∧ valid_u64 s'.total_borrows ∧ valid_u64 s'.interest_rate := sorry

end Lending
