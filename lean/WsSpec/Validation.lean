import WsSpec.Provider

/-!
# Request validation

The bounds Cloudflare documents for a search request, and the proof that a
request built through `mkRequest` always satisfies them. Mirrors
`src/validate.rs`. (The Rust code is stricter in one respect: it also rejects
whitespace-only queries.)
-/

namespace WsSpec

/-- Result limits accepted by the API: 1 to 10 inclusive. -/
def limitValid (n : Nat) : Bool := 1 ≤ n && n ≤ 10

/-- The limit `ws` sends when none is configured. -/
def defaultLimit : Nat := 10

/-- Query lengths accepted by the API, in characters: 1 to 1,024 inclusive. -/
def queryLenValid (n : Nat) : Bool := 1 ≤ n && n ≤ 1024

/-- A query is valid when its length *in characters* (not bytes) is in bounds. -/
def queryValid (q : String) : Bool := queryLenValid q.length

theorem limitValid_iff (n : Nat) : limitValid n = true ↔ 1 ≤ n ∧ n ≤ 10 := by
  simp [limitValid]

theorem queryLenValid_iff (n : Nat) : queryLenValid n = true ↔ 1 ≤ n ∧ n ≤ 1024 := by
  simp [queryLenValid]

/-- The default limit is itself valid. -/
theorem defaultLimit_valid : limitValid defaultLimit = true := by decide

/-- Exactly ten limits are valid, namely 1 through 10. -/
theorem valid_limits : (List.range 100).filter limitValid = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10] := by
  decide

/-- Boundary behaviour, as tested in Rust. -/
example : limitValid 0 = false := by decide
example : limitValid 1 = true := by decide
example : limitValid 10 = true := by decide
example : limitValid 11 = false := by decide
example : queryLenValid 0 = false := by decide
example : queryLenValid 1 = true := by decide
example : queryLenValid 1024 = true := by decide
example : queryLenValid 1025 = false := by decide

/-- The empty query is rejected. -/
theorem empty_query_invalid : queryValid "" = false := by decide

/-- Length is counted in characters: a four-byte emoji is one character. -/
example : "🦀".length = 1 ∧ "🦀".utf8ByteSize = 4 := by decide

/-- A search request as sent to Cloudflare. -/
structure Request where
  query : String
  provider : Provider
  limit : Nat
  gateway : String

/-- The only way `ws` builds a request: validation first. -/
def mkRequest (query : String) (provider : Provider) (limit : Nat) (gateway : String) :
    Option Request :=
  if queryValid query && limitValid limit && gateway != "" then
    some { query, provider, limit, gateway }
  else
    none

/-- Soundness: every request that is built is within the API's documented bounds. -/
theorem mkRequest_sound {q : String} {p : Provider} {l : Nat} {g : String} {r : Request}
    (h : mkRequest q p l g = some r) :
    1 ≤ r.query.length ∧ r.query.length ≤ 1024 ∧ 1 ≤ r.limit ∧ r.limit ≤ 10 ∧ r.gateway ≠ "" := by
  unfold mkRequest at h
  split at h
  · rename_i hv
    cases h
    simp [queryValid, queryLenValid, limitValid] at hv
    refine ⟨?_, ?_, ?_, ?_, hv.2⟩ <;> dsimp only <;> omega
  · cases h

/-- Validation never alters what the user asked for. -/
theorem mkRequest_preserves {q : String} {p : Provider} {l : Nat} {g : String} {r : Request}
    (h : mkRequest q p l g = some r) :
    r.query = q ∧ r.provider = p ∧ r.limit = l ∧ r.gateway = g := by
  unfold mkRequest at h
  split at h
  · cases h; exact ⟨rfl, rfl, rfl, rfl⟩
  · cases h

/-- Completeness: valid input is never refused. -/
theorem mkRequest_complete (q : String) (p : Provider) (l : Nat) (g : String)
    (hq : queryValid q = true) (hl : limitValid l = true) (hg : g ≠ "") :
    (mkRequest q p l g).isSome = true := by
  simp [mkRequest, hq, hl, hg]

/-- Invalid limits never produce a request, so they are never billed. -/
theorem mkRequest_rejects_bad_limit (q : String) (p : Provider) (l : Nat) (g : String)
    (hl : limitValid l = false) : mkRequest q p l g = none := by
  simp [mkRequest, hl]

end WsSpec
