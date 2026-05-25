import QEDGen.Solana.Account
import QEDGen.Solana.Cpi
import QEDGen.Solana.State
import QEDGen.Solana.Valid

namespace Multisig

open QEDGen.Solana

abbrev MAX_MEMBERS : Nat := 32

inductive Status where
  | Uninitialized
  | Active
  | HasProposal
  deriving Repr, DecidableEq, BEq

structure State where
  creator : Pubkey
  threshold : Nat
  member_count : Nat
  members : Map[MAX_MEMBERS] Pubkey
  voted : Map[MAX_MEMBERS] U8
  approval_count : Nat
  rejection_count : Nat
  status : Status
  deriving Repr, DecidableEq, BEq

def create_vaultTransition (s : State) (signer : Pubkey) (threshold : Nat) (member_count : Nat) : Option State :=
  if signer = s.creator ∧ s.status = .Uninitialized ∧ threshold > 0 ∧ threshold ≤ member_count ∧ member_count ≤ 32 then
    some { s with threshold := threshold, member_count := member_count, approval_count := 0, rejection_count := 0, status := .Active }
  else none

def proposeTransition (s : State) (signer : Pubkey) : Option State :=
  if signer = s.creator ∧ s.status = .Active then
    some { s with approval_count := 0, rejection_count := 0, status := .HasProposal }
  else none

def approveTransition (s : State) (signer : Pubkey) (member_index : Nat) : Option State :=
  let approver := signer
  if s.status = .HasProposal ∧ member_index < s.member_count ∧ s.members[member_index] = approver ∧ s.voted[member_index] = 0 ∧ s.approval_count + 1 ≤ 255 then
    some { s with approval_count := s.approval_count + 1, voted[member_index] := 1, status := .HasProposal }
  else none

def rejectTransition (s : State) (signer : Pubkey) (member_index : Nat) : Option State :=
  let rejecter := signer
  if s.status = .HasProposal ∧ member_index < s.member_count ∧ s.members[member_index] = rejecter ∧ s.voted[member_index] = 0 ∧ s.rejection_count + 1 ≤ 255 then
    some { s with rejection_count := s.rejection_count + 1, voted[member_index] := 1, status := .HasProposal }
  else none

def executeTransition (s : State) (signer : Pubkey) (member_index : Nat) : Option State :=
  let executor := signer
  if s.status = .HasProposal ∧ member_index < s.member_count ∧ s.members[member_index] = executor ∧ s.approval_count ≥ s.threshold then
    some { s with approval_count := 0, rejection_count := 0, status := .Active }
  else none

def cancel_proposalTransition (s : State) (signer : Pubkey) : Option State :=
  if s.status = .HasProposal ∧ s.member_count - s.rejection_count < s.threshold then
    some { s with approval_count := 0, rejection_count := 0, status := .Active }
  else none

def add_memberTransition (s : State) (signer : Pubkey) (member_index : Nat) (member_pubkey : Pubkey) : Option State :=
  if signer = s.creator ∧ s.status = .Active ∧ member_index < s.member_count then
    some { s with members[member_index] := member_pubkey, status := .Active }
  else none

def remove_memberTransition (s : State) (signer : Pubkey) : Option State :=
  if signer = s.creator ∧ s.status = .Active ∧ 1 ≤ s.member_count then
    some { s with member_count := s.member_count - 1, status := .Active }
  else none

inductive Operation where
  | create_vault (threshold : Nat) (member_count : Nat)
  | propose
  | approve (member_index : Nat)
  | reject (member_index : Nat)
  | execute (member_index : Nat)
  | cancel_proposal
  | add_member (member_index : Nat) (member_pubkey : Pubkey)
  | remove_member
  deriving Repr, DecidableEq, BEq

def applyOp (s : State) (signer : Pubkey) : Operation → Option State
  | .create_vault threshold member_count => create_vaultTransition s signer threshold member_count
  | .propose => proposeTransition s signer
  | .approve member_index => approveTransition s signer member_index
  | .reject member_index => rejectTransition s signer member_index
  | .execute member_index => executeTransition s signer member_index
  | .cancel_proposal => cancel_proposalTransition s signer
  | .add_member member_index member_pubkey => add_memberTransition s signer member_index member_pubkey
  | .remove_member => remove_memberTransition s signer

def threshold_bounded (s : State) : Prop := s.threshold ≤ s.member_count ∧ s.threshold > 0

theorem threshold_bounded_preserved_by_create_vault (s s' : State) (signer : Pubkey) (threshold : Nat) (member_count : Nat)
    (h_inv : threshold_bounded s) (h : create_vaultTransition s signer threshold member_count = some s') :
    threshold_bounded s' := sorry

theorem threshold_bounded_preserved_by_propose (s s' : State) (signer : Pubkey)
    (h_inv : threshold_bounded s) (h : proposeTransition s signer = some s') :
    threshold_bounded s' := sorry

theorem threshold_bounded_preserved_by_approve (s s' : State) (signer : Pubkey) (member_index : Nat)
    (h_inv : threshold_bounded s) (h : approveTransition s signer member_index = some s') :
    threshold_bounded s' := sorry

theorem threshold_bounded_preserved_by_reject (s s' : State) (signer : Pubkey) (member_index : Nat)
    (h_inv : threshold_bounded s) (h : rejectTransition s signer member_index = some s') :
    threshold_bounded s' := sorry

theorem threshold_bounded_preserved_by_execute (s s' : State) (signer : Pubkey) (member_index : Nat)
    (h_inv : threshold_bounded s) (h : executeTransition s signer member_index = some s') :
    threshold_bounded s' := sorry

theorem threshold_bounded_preserved_by_cancel_proposal (s s' : State) (signer : Pubkey)
    (h_inv : threshold_bounded s) (h : cancel_proposalTransition s signer = some s') :
    threshold_bounded s' := sorry

theorem threshold_bounded_preserved_by_add_member (s s' : State) (signer : Pubkey) (member_index : Nat) (member_pubkey : Pubkey)
    (h_inv : threshold_bounded s) (h : add_memberTransition s signer member_index member_pubkey = some s') :
    threshold_bounded s' := sorry

theorem threshold_bounded_preserved_by_remove_member (s s' : State) (signer : Pubkey)
    (h_inv : threshold_bounded s) (h : remove_memberTransition s signer = some s') :
    threshold_bounded s' := sorry

/-- threshold_bounded is preserved by every operation. -/
theorem threshold_bounded_invariant (s s' : State) (signer : Pubkey) (op : Operation)
    (h_inv : threshold_bounded s) (h : applyOp s signer op = some s') :
    threshold_bounded s' := sorry

def votes_bounded (s : State) : Prop := s.approval_count + s.rejection_count ≤ s.member_count

theorem votes_bounded_preserved_by_create_vault (s s' : State) (signer : Pubkey) (threshold : Nat) (member_count : Nat)
    (h_inv : votes_bounded s) (h : create_vaultTransition s signer threshold member_count = some s') :
    votes_bounded s' := sorry

theorem votes_bounded_preserved_by_propose (s s' : State) (signer : Pubkey)
    (h_inv : votes_bounded s) (h : proposeTransition s signer = some s') :
    votes_bounded s' := sorry

theorem votes_bounded_preserved_by_execute (s s' : State) (signer : Pubkey) (member_index : Nat)
    (h_inv : votes_bounded s) (h : executeTransition s signer member_index = some s') :
    votes_bounded s' := sorry

theorem votes_bounded_preserved_by_cancel_proposal (s s' : State) (signer : Pubkey)
    (h_inv : votes_bounded s) (h : cancel_proposalTransition s signer = some s') :
    votes_bounded s' := sorry

theorem votes_bounded_preserved_by_remove_member (s s' : State) (signer : Pubkey)
    (h_inv : votes_bounded s) (h : remove_memberTransition s signer = some s') :
    votes_bounded s' := sorry

/-- votes_bounded is preserved by every operation. -/
theorem votes_bounded_invariant (s s' : State) (signer : Pubkey) (op : Operation)
    (h_inv : votes_bounded s) (h : applyOp s signer op = some s') :
    votes_bounded s' := sorry

-- ============================================================================
-- Abort conditions — operations must reject under specified conditions
-- ============================================================================

theorem create_vault_aborts_if_InvalidThreshold (s : State) (signer : Pubkey) (threshold : Nat) (member_count : Nat)
    (h : ¬(threshold > 0 ∧ threshold ≤ member_count)) : create_vaultTransition s signer threshold member_count = none := by
  unfold create_vaultTransition
  rw [if_neg (fun hg => h ⟨hg.2.2.1, hg.2.2.2.1⟩)]

theorem create_vault_aborts_if_TooManyMembers (s : State) (signer : Pubkey) (threshold : Nat) (member_count : Nat)
    (h : ¬(member_count ≤ 32)) : create_vaultTransition s signer threshold member_count = none := by
  unfold create_vaultTransition
  rw [if_neg (fun hg => h hg.2.2.2.2)]

theorem approve_aborts_if_NotAMember_0 (s : State) (signer : Pubkey) (member_index : Nat)
    (h : ¬(member_index < s.member_count)) : approveTransition s signer member_index = none := by
  unfold approveTransition
  rw [if_neg (fun hg => h hg.2.1)]

theorem approve_aborts_if_NotAMember_1 (s : State) (signer : Pubkey) (member_index : Nat)
    (h : ¬(s.members[member_index] = approver)) : approveTransition s signer member_index = none := by
  unfold approveTransition
  rw [if_neg (fun hg => h hg.2.2.1)]

theorem approve_aborts_if_AlreadyVoted (s : State) (signer : Pubkey) (member_index : Nat)
    (h : ¬(s.voted[member_index] = 0)) : approveTransition s signer member_index = none := by
  unfold approveTransition
  rw [if_neg (fun hg => h hg.2.2.2.1)]

theorem reject_aborts_if_NotAMember_0 (s : State) (signer : Pubkey) (member_index : Nat)
    (h : ¬(member_index < s.member_count)) : rejectTransition s signer member_index = none := by
  unfold rejectTransition
  rw [if_neg (fun hg => h hg.2.1)]

theorem reject_aborts_if_NotAMember_1 (s : State) (signer : Pubkey) (member_index : Nat)
    (h : ¬(s.members[member_index] = rejecter)) : rejectTransition s signer member_index = none := by
  unfold rejectTransition
  rw [if_neg (fun hg => h hg.2.2.1)]

theorem reject_aborts_if_AlreadyVoted (s : State) (signer : Pubkey) (member_index : Nat)
    (h : ¬(s.voted[member_index] = 0)) : rejectTransition s signer member_index = none := by
  unfold rejectTransition
  rw [if_neg (fun hg => h hg.2.2.2.1)]

theorem execute_aborts_if_NotAMember_0 (s : State) (signer : Pubkey) (member_index : Nat)
    (h : ¬(member_index < s.member_count)) : executeTransition s signer member_index = none := by
  unfold executeTransition
  rw [if_neg (fun hg => h hg.2.1)]

theorem execute_aborts_if_NotAMember_1 (s : State) (signer : Pubkey) (member_index : Nat)
    (h : ¬(s.members[member_index] = executor)) : executeTransition s signer member_index = none := by
  unfold executeTransition
  rw [if_neg (fun hg => h hg.2.2.1)]

theorem execute_aborts_if_ThresholdNotMet (s : State) (signer : Pubkey) (member_index : Nat)
    (h : ¬(s.approval_count ≥ s.threshold)) : executeTransition s signer member_index = none := by
  unfold executeTransition
  rw [if_neg (fun hg => h hg.2.2.2)]

theorem cancel_proposal_aborts_if_ThresholdUnreachable (s : State) (signer : Pubkey)
    (h : ¬(s.member_count - s.rejection_count < s.threshold)) : cancel_proposalTransition s signer = none := by
  unfold cancel_proposalTransition
  rw [if_neg (fun hg => h hg.2)]

theorem add_member_aborts_if_NotAMember (s : State) (signer : Pubkey) (member_index : Nat) (member_pubkey : Pubkey)
    (h : ¬(member_index < s.member_count)) : add_memberTransition s signer member_index member_pubkey = none := by
  unfold add_memberTransition
  rw [if_neg (fun hg => h hg.2.2)]

-- ============================================================================
-- Cover properties — reachability (existential proofs)
-- ============================================================================

/-- proposal_lifecycle — trace [create_vault, propose, approve, execute] is reachable. -/
theorem cover_proposal_lifecycle : ∃ (s0 : State) (signer : Pubkey),
    ∃ (v0_0 : Nat) (v0_1 : Nat), ∃ (s1 : State), create_vaultTransition s0 signer v0_0 v0_1 = some s1 ∧
∃ (s2 : State), proposeTransition s1 signer = some s2 ∧
        ∃ (v2_0 : Nat), ∃ (s3 : State), approveTransition s2 signer v2_0 = some s3 ∧
          ∃ (v3_0 : Nat), executeTransition s3 signer v3_0 ≠ none := by
  let pk : Pubkey := ⟨0, 0, 0, 0⟩
  let s0 : State := ⟨pk, 0, 0, 0, 0, 0, 0, .Uninitialized⟩
  let s1 : State := ⟨pk, 1, 1, 0, 0, 0, 0, .Active⟩
  let s2 : State := ⟨pk, 1, 1, 0, 0, 0, 0, .HasProposal⟩
  let s3 : State := ⟨pk, 1, 1, 0, 0, 1, 0, .HasProposal⟩
  exact ⟨s0, pk, 1, 1, s1, by decide, s2, by decide, 0, s3, by decide, 0, by decide⟩

/-- rejection_flow — trace [create_vault, propose, reject, cancel_proposal] is reachable. -/
theorem cover_rejection_flow : ∃ (s0 : State) (signer : Pubkey),
    ∃ (v0_0 : Nat) (v0_1 : Nat), ∃ (s1 : State), create_vaultTransition s0 signer v0_0 v0_1 = some s1 ∧
∃ (s2 : State), proposeTransition s1 signer = some s2 ∧
        ∃ (v2_0 : Nat), ∃ (s3 : State), rejectTransition s2 signer v2_0 = some s3 ∧
cancel_proposalTransition s3 signer ≠ none := by
  let pk : Pubkey := ⟨0, 0, 0, 0⟩
  let s0 : State := ⟨pk, 0, 0, 0, 0, 0, 0, .Uninitialized⟩
  let s1 : State := ⟨pk, 1, 1, 0, 0, 0, 0, .Active⟩
  let s2 : State := ⟨pk, 1, 1, 0, 0, 0, 0, .HasProposal⟩
  let s3 : State := ⟨pk, 1, 1, 0, 0, 0, 1, .HasProposal⟩
  exact ⟨s0, pk, 1, 1, s1, by decide, s2, by decide, 0, s3, by decide, by decide⟩

-- ============================================================================
-- Liveness properties — bounded reachability (leads-to)
-- ============================================================================

def applyOps (s : State) (signer : Pubkey) : List Operation → Option State
  | [] => some s
  | op :: ops => match applyOp s signer op with
    | some s' => applyOps s' signer ops
    | none => none

/-- proposal_resolves — from HasProposal leads to Active within 1 steps via [execute, cancel_proposal]. -/
theorem liveness_proposal_resolves (s : State) (signer : Pubkey)
    (h : s.status = .HasProposal) :
    ∃ ops, ops.length ≤ 1 ∧ ∀ s', applyOps s signer ops = some s' → s'.status = .Active := by
  refine ⟨[.execute 0], by decide, fun s' h_apply => ?_⟩
  simp only [applyOps, applyOp, executeTransition] at h_apply
  split at h_apply
  · next heq =>
    split at heq
    · next hg => simp at heq h_apply; subst heq; subst h_apply; rfl
    · simp at heq
  · simp at h_apply

-- ============================================================================
-- Overflow safety obligations (auto-generated for operations with add effects)
-- ============================================================================

theorem approve_overflow_safe (s s' : State) (signer : Pubkey) (member_index : Nat)
    (h_valid : valid_u8 s.threshold ∧ valid_u8 s.member_count ∧ valid_u8 s.approval_count ∧ valid_u8 s.rejection_count)
    (h_inv_threshold_bounded : threshold_bounded s)
    (h : approveTransition s signer member_index = some s') :
    valid_u8 s'.threshold ∧ valid_u8 s'.member_count ∧ valid_u8 s'.approval_count ∧ valid_u8 s'.rejection_count := sorry

theorem reject_overflow_safe (s s' : State) (signer : Pubkey) (member_index : Nat)
    (h_valid : valid_u8 s.threshold ∧ valid_u8 s.member_count ∧ valid_u8 s.approval_count ∧ valid_u8 s.rejection_count)
    (h_inv_threshold_bounded : threshold_bounded s)
    (h : rejectTransition s signer member_index = some s') :
    valid_u8 s'.threshold ∧ valid_u8 s'.member_count ∧ valid_u8 s'.approval_count ∧ valid_u8 s'.rejection_count := sorry

end Multisig
