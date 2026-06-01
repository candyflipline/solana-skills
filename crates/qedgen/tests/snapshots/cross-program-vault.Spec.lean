import QEDGen.Solana.Account
import QEDGen.Solana.Cpi
import QEDGen.Solana.State
import QEDGen.Solana.Valid
import Token

namespace Vault

open QEDGen.Solana

inductive Status where
  | Uninitialized
  | Active
  deriving Repr, DecidableEq, BEq

inductive State where
  | Uninitialized
  | Active (total_deposits : Nat)
  deriving Repr, DecidableEq, BEq

instance : Inhabited State := ⟨.Uninitialized⟩

def State.status : State → Status
  | .Uninitialized => .Uninitialized
  | .Active _ => .Active

def State.total_deposits : State → Nat
  | .Uninitialized => 0
  | .Active total_deposits => total_deposits

def initializeTransition (s : State) (signer : Pubkey) : Option State :=
  match s with
  | .Uninitialized => some (.Active 0)
  | _ => none

def depositTransition (s : State) (signer : Pubkey) (amount : Nat) : Option State :=
  match s with
  | .Active total_deposits =>
    if amount > 0 ∧ total_deposits + amount ≤ 18446744073709551615 then some (.Active (total_deposits + amount)) else none
  | _ => none

def emergency_closeTransition (s : State) (signer : Pubkey) : Option State :=
  match s with
  | .Active total_deposits =>
    let admin := signer
    some (.Active 0)
  | _ => none

-- `Token.transfer` ensures #0 (from_balance): caller supplied no `state_binders` for these abstract fields; ensures not pulled into caller proof. Bind via `state_binders { from_balance = state.<field> }` to consume.
-- `Token.transfer` ensures #1 (to_balance): caller supplied no `state_binders` for these abstract fields; ensures not pulled into caller proof. Bind via `state_binders { to_balance = state.<field> }` to consume.
-- `Token.transfer` ensures #0 (from_balance): caller supplied no `state_binders` for these abstract fields; ensures not pulled into caller proof. Bind via `state_binders { from_balance = state.<field> }` to consume.
-- `Token.transfer` ensures #1 (to_balance): caller supplied no `state_binders` for these abstract fields; ensures not pulled into caller proof. Bind via `state_binders { to_balance = state.<field> }` to consume.
inductive Operation where
  | «initialize»
  | deposit (amount : Nat)
  | emergency_close
  deriving Repr, DecidableEq, BEq

def applyOp (s : State) (signer : Pubkey) : Operation → Option State
  | .«initialize» => initializeTransition s signer
  | .deposit amount => depositTransition s signer amount
  | .emergency_close => emergency_closeTransition s signer

-- ============================================================================
-- Abort conditions — operations must reject under specified conditions
-- ============================================================================

theorem deposit_aborts_if_InvalidAmount (s : State) (signer : Pubkey) (amount : Nat)
    (h : ¬(amount > 0)) : depositTransition s signer amount = none := by
  unfold depositTransition
  cases s <;> simp_all

-- ============================================================================
-- Frame conditions (modifies)
-- ============================================================================

theorem deposit_frame (s s' : State) (signer : Pubkey) (amount : Nat)
    (h : depositTransition s signer amount = some s') :
    -- todo!(): inductive-State frame condition. Statement needs
    -- per-pre-variant case analysis to express which payload
    -- fields are preserved. Stated as `True` until that lands;
    -- the honest placeholder proof is `trivial`, not `sorry`.
    True := trivial

-- ============================================================================
-- Overflow safety obligations (auto-generated for operations with add effects)
-- ============================================================================

theorem deposit_overflow_safe (s s' : State) (signer : Pubkey) (amount : Nat)
    (h_valid : valid_u64 s.total_deposits)
    (h : depositTransition s signer amount = some s') :
    valid_u64 s'.total_deposits := by sorry

end Vault
