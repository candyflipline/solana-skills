import QEDGen.Solana.Account
import QEDGen.Solana.Cpi
import QEDGen.Solana.State
import QEDGen.Solana.Valid

namespace Escrow

open QEDGen.Solana

inductive Status where
  | Uninitialized
  | Open
  | Closed
  deriving Repr, DecidableEq, BEq

inductive State where
  | Uninitialized
  | Open (initializer : Pubkey) (initializer_token_account : Pubkey) (taker : Pubkey) (initializer_amount : Nat) (taker_amount : Nat) (escrow_token_account : Pubkey)
  | Closed
  deriving Repr, DecidableEq, BEq

def State.status : State → Status
  | .Uninitialized => .Uninitialized
  | .Open _ _ _ _ _ _ => .Open
  | .Closed => .Closed

def State.initializer : State → Pubkey
  | .Uninitialized => default
  | .Open initializer _ _ _ _ _ => initializer
  | .Closed => default

def State.initializer_token_account : State → Pubkey
  | .Uninitialized => default
  | .Open _ initializer_token_account _ _ _ _ => initializer_token_account
  | .Closed => default

def State.taker : State → Pubkey
  | .Uninitialized => default
  | .Open _ _ taker _ _ _ => taker
  | .Closed => default

def State.initializer_amount : State → Nat
  | .Uninitialized => 0
  | .Open _ _ _ initializer_amount _ _ => initializer_amount
  | .Closed => 0

def State.taker_amount : State → Nat
  | .Uninitialized => 0
  | .Open _ _ _ _ taker_amount _ => taker_amount
  | .Closed => 0

def State.escrow_token_account : State → Pubkey
  | .Uninitialized => default
  | .Open _ _ _ _ _ escrow_token_account => escrow_token_account
  | .Closed => default

def initializeTransition (s : State) (signer : Pubkey) (deposit_amount : Nat) (receive_amount : Nat) : Option State :=
  -- todo!(): post-variant `Open` has unconstrained field(s) not derivable from spec: initializer_token_account, taker, escrow_token_account
  -- Using type defaults; add effects or handler params to constrain these.
  match s with
  | .Uninitialized =>
    let initializer := signer
    if deposit_amount > 0 ∧ receive_amount > 0 then some (.Open initializer default default deposit_amount receive_amount default) else none
  | _ => none

def exchangeTransition (s : State) (signer : Pubkey) : Option State :=
  match s with
  | .Open initializer initializer_token_account taker initializer_amount taker_amount escrow_token_account =>
    if signer = taker then some (.Closed) else none
  | _ => none

def cancelTransition (s : State) (signer : Pubkey) : Option State :=
  match s with
  | .Open initializer initializer_token_account taker initializer_amount taker_amount escrow_token_account =>
    if signer = initializer then some (.Closed) else none
  | _ => none

/-- initialize transfer envelope: initializer_ta → escrow_ta amount deposit_amount authority initializer.
    Verifies CPI shape (program ID, account list, discriminator).
    Amount serialization and SPL Token execution are SDK/runtime
    trust per VERIFICATION_SCOPE.md. -/
def build_initialize_transfer (from_pk to_pk authority_pk : Pubkey) : CpiInstruction :=
  { programId := TOKEN_PROGRAM_ID
  , accounts :=
      [ ⟨from_pk, false, true⟩
      , ⟨to_pk, false, true⟩
      , ⟨authority_pk, true, false⟩
      ]
  , data := DISC_TRANSFER }

theorem initialize_transfer_correct (from_pk to_pk authority_pk : Pubkey) :
    let cpi := build_initialize_transfer from_pk to_pk authority_pk
    targetsProgram cpi TOKEN_PROGRAM_ID ∧
    accountAt cpi 0 from_pk false true ∧
    accountAt cpi 1 to_pk false true ∧
    accountAt cpi 2 authority_pk true false ∧
    hasDiscriminator cpi DISC_TRANSFER := by
  unfold build_initialize_transfer targetsProgram accountAt hasDiscriminator
  exact ⟨rfl, rfl, rfl, rfl, rfl⟩

/-- exchange transfer envelope: taker_ta → initializer_ta amount taker_amount authority taker.
    Verifies CPI shape (program ID, account list, discriminator).
    Amount serialization and SPL Token execution are SDK/runtime
    trust per VERIFICATION_SCOPE.md. -/
def build_exchange_transfer_0 (from_pk to_pk authority_pk : Pubkey) : CpiInstruction :=
  { programId := TOKEN_PROGRAM_ID
  , accounts :=
      [ ⟨from_pk, false, true⟩
      , ⟨to_pk, false, true⟩
      , ⟨authority_pk, true, false⟩
      ]
  , data := DISC_TRANSFER }

theorem exchange_transfer_0_correct (from_pk to_pk authority_pk : Pubkey) :
    let cpi := build_exchange_transfer_0 from_pk to_pk authority_pk
    targetsProgram cpi TOKEN_PROGRAM_ID ∧
    accountAt cpi 0 from_pk false true ∧
    accountAt cpi 1 to_pk false true ∧
    accountAt cpi 2 authority_pk true false ∧
    hasDiscriminator cpi DISC_TRANSFER := by
  unfold build_exchange_transfer_0 targetsProgram accountAt hasDiscriminator
  exact ⟨rfl, rfl, rfl, rfl, rfl⟩

/-- exchange transfer envelope: escrow_ta → taker_ta amount initializer_amount authority escrow.
    Verifies CPI shape (program ID, account list, discriminator).
    Amount serialization and SPL Token execution are SDK/runtime
    trust per VERIFICATION_SCOPE.md. -/
def build_exchange_transfer_1 (from_pk to_pk authority_pk : Pubkey) : CpiInstruction :=
  { programId := TOKEN_PROGRAM_ID
  , accounts :=
      [ ⟨from_pk, false, true⟩
      , ⟨to_pk, false, true⟩
      , ⟨authority_pk, true, false⟩
      ]
  , data := DISC_TRANSFER }

theorem exchange_transfer_1_correct (from_pk to_pk authority_pk : Pubkey) :
    let cpi := build_exchange_transfer_1 from_pk to_pk authority_pk
    targetsProgram cpi TOKEN_PROGRAM_ID ∧
    accountAt cpi 0 from_pk false true ∧
    accountAt cpi 1 to_pk false true ∧
    accountAt cpi 2 authority_pk true false ∧
    hasDiscriminator cpi DISC_TRANSFER := by
  unfold build_exchange_transfer_1 targetsProgram accountAt hasDiscriminator
  exact ⟨rfl, rfl, rfl, rfl, rfl⟩

/-- cancel transfer envelope: escrow_ta → initializer_ta amount initializer_amount authority escrow.
    Verifies CPI shape (program ID, account list, discriminator).
    Amount serialization and SPL Token execution are SDK/runtime
    trust per VERIFICATION_SCOPE.md. -/
def build_cancel_transfer (from_pk to_pk authority_pk : Pubkey) : CpiInstruction :=
  { programId := TOKEN_PROGRAM_ID
  , accounts :=
      [ ⟨from_pk, false, true⟩
      , ⟨to_pk, false, true⟩
      , ⟨authority_pk, true, false⟩
      ]
  , data := DISC_TRANSFER }

theorem cancel_transfer_correct (from_pk to_pk authority_pk : Pubkey) :
    let cpi := build_cancel_transfer from_pk to_pk authority_pk
    targetsProgram cpi TOKEN_PROGRAM_ID ∧
    accountAt cpi 0 from_pk false true ∧
    accountAt cpi 1 to_pk false true ∧
    accountAt cpi 2 authority_pk true false ∧
    hasDiscriminator cpi DISC_TRANSFER := by
  unfold build_cancel_transfer targetsProgram accountAt hasDiscriminator
  exact ⟨rfl, rfl, rfl, rfl, rfl⟩

-- INVARIANT OBLIGATION (declared, no predicate body): conservation
--   description: total tokens preserved across initialize, exchange, cancel
-- The spec declared this name but didn't supply a predicate body
-- (`invariant <name> : <expr>`). The codegen has no goal to lower —
-- pre-v2.14 emitted `theorem <name> : True := trivial`, which
-- was tautological. To verify this invariant, give it a body in
-- the spec.

inductive Operation where
  | «initialize» (deposit_amount : Nat) (receive_amount : Nat)
  | exchange
  | cancel
  deriving Repr, DecidableEq, BEq

def applyOp (s : State) (signer : Pubkey) : Operation → Option State
  | .«initialize» deposit_amount receive_amount => initializeTransition s signer deposit_amount receive_amount
  | .exchange => exchangeTransition s signer
  | .cancel => cancelTransition s signer

-- ============================================================================
-- Abort conditions — operations must reject under specified conditions
-- ============================================================================

theorem initialize_aborts_if_InvalidAmount (s : State) (signer : Pubkey) (deposit_amount : Nat) (receive_amount : Nat)
    (h : ¬(deposit_amount > 0 ∧ receive_amount > 0)) : initializeTransition s signer deposit_amount receive_amount = none := by sorry

-- ============================================================================
-- Cover properties — reachability (existential proofs)
-- ============================================================================

/-- happy_path — trace [initialize, exchange] is reachable. -/
theorem cover_happy_path : ∃ (s0 : State) (signer : Pubkey),
    ∃ (v0_0 : Nat) (v0_1 : Nat), ∃ (s1 : State), initializeTransition s0 signer v0_0 v0_1 = some s1 ∧
exchangeTransition s1 signer ≠ none := by
  let pk : Pubkey := ⟨0, 0, 0, 0⟩
  let s0 : State := (.Uninitialized : State)
  let s1 : State := (.Open pk pk pk 1 1 pk : State)
  exact ⟨s0, pk, 1, 1, s1, by decide, by decide⟩

/-- cancel_path — trace [initialize, cancel] is reachable. -/
theorem cover_cancel_path : ∃ (s0 : State) (signer : Pubkey),
    ∃ (v0_0 : Nat) (v0_1 : Nat), ∃ (s1 : State), initializeTransition s0 signer v0_0 v0_1 = some s1 ∧
cancelTransition s1 signer ≠ none := by
  let pk : Pubkey := ⟨0, 0, 0, 0⟩
  let s0 : State := (.Uninitialized : State)
  let s1 : State := (.Open pk pk pk 1 1 pk : State)
  exact ⟨s0, pk, 1, 1, s1, by decide, by decide⟩

-- ============================================================================
-- Liveness properties — bounded reachability (leads-to)
-- ============================================================================

def applyOps (s : State) (signer : Pubkey) : List Operation → Option State
  | [] => some s
  | op :: ops => match applyOp s signer op with
    | some s' => applyOps s' signer ops
    | none => none

/-- escrow_settles — from Open leads to Closed within 1 steps via [exchange, cancel]. -/
theorem liveness_escrow_settles (s : State) (signer : Pubkey)
    (h : s.status = .Open) :
    ∃ ops, ops.length ≤ 1 ∧ ∀ s', applyOps s signer ops = some s' → s'.status = .Closed := by sorry

end Escrow
