import Lake
open Lake DSL

package escrow_splitProofs

-- v2.27 Track B: verified-callee proof package (Stance 2).
require tokenProofs from "../../../../../../.qedgen/cache/builtin/spl/.qed/proofs"

require qedgenSupport from
  "./lean_solana"

@[default_target]
lean_lib Escrow_splitSpec where
  roots := #[`Spec, `Proofs]
