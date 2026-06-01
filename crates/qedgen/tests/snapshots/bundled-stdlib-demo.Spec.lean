import QEDGen.Solana.Account
import QEDGen.Solana.Cpi
import QEDGen.Solana.State
import QEDGen.Solana.Valid
import Token

namespace PoolDemo

open QEDGen.Solana

inductive Status where
  | Uninitialized
  | Open
  deriving Repr, DecidableEq, BEq, Inhabited

structure State where
  pool_balance : Nat
  status : Status
  deriving Repr, DecidableEq, BEq, Inhabited

def initializeTransition (s : State) (signer : Pubkey) (initial : Nat) : Option State :=
  if s.status = .Uninitialized ∧ initial > 0 then
    some { s with pool_balance := initial, status := .Open }
  else none

def depositTransition (s : State) (signer : Pubkey) (amount : Nat) : Option State :=
  if s.status = .Open ∧ amount > 0 ∧ s.pool_balance + amount ≤ 18446744073709551615 then
    some { s with pool_balance := s.pool_balance + amount, status := .Open }
  else none

-- `Token.transfer` ensures #0 (from_balance): caller supplied no `state_binders` for these abstract fields; ensures not pulled into caller proof. Bind via `state_binders { from_balance = state.<field> }` to consume.
/-- Token.transfer.ensures @ `deposit` call #0 (stance 2: discharged via imported callee proof). -/
theorem deposit_Token_transfer_call_0_post_1 (s : State) (pre post : State) (amount : Nat) : post.pool_balance = pre.pool_balance + amount :=
  Token.transfer.ensures_axiom_1 pre post amount (·.pool_balance)

inductive Operation where
  | «initialize» (initial : Nat)
  | deposit (amount : Nat)
  deriving Repr, DecidableEq, BEq

def applyOp (s : State) (signer : Pubkey) : Operation → Option State
  | .«initialize» initial => initializeTransition s signer initial
  | .deposit amount => depositTransition s signer amount

-- ============================================================================
-- Abort conditions — operations must reject under specified conditions
-- ============================================================================

theorem initialize_aborts_if_InvalidAmount (s : State) (signer : Pubkey) (initial : Nat)
    (h : ¬(initial > 0)) : initializeTransition s signer initial = none := by
  unfold initializeTransition
  rw [if_neg (fun hg => h hg.2)]

theorem deposit_aborts_if_InvalidAmount (s : State) (signer : Pubkey) (amount : Nat)
    (h : ¬(amount > 0)) : depositTransition s signer amount = none := by
  unfold depositTransition
  rw [if_neg (fun hg => h hg.2.1)]

-- ============================================================================
-- Frame conditions (modifies)
-- ============================================================================

-- ============================================================================
-- Overflow safety obligations (auto-generated for operations with add effects)
-- ============================================================================

theorem deposit_overflow_safe (s s' : State) (signer : Pubkey) (amount : Nat)
    (h_valid : valid_u64 s.pool_balance)
    (h : depositTransition s signer amount = some s') :
    valid_u64 s'.pool_balance := by
  unfold depositTransition at h; split at h
  · next hg =>
    cases h
    simp only [valid_u64, Valid.valid_u64, Valid.U64_MAX]; omega
  · contradiction

end PoolDemo
