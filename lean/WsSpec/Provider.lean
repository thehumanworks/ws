/-!
# Providers

The three web search providers behind Cloudflare's AI Gateway Web Search API
and their wire names. Mirrors `src/provider.rs`.
-/

namespace WsSpec

/-- A web search provider. -/
inductive Provider where
  | ceramic
  | exa
  | linkup
  deriving DecidableEq, Repr

namespace Provider

/-- The name sent on the wire. -/
def name : Provider → String
  | ceramic => "ceramic"
  | exa => "exa"
  | linkup => "linkup"

/-- Parse a wire name. -/
def parse (s : String) : Option Provider :=
  if s = "ceramic" then some ceramic
  else if s = "exa" then some exa
  else if s = "linkup" then some linkup
  else none

/-- Every provider. -/
def all : List Provider := [ceramic, exa, linkup]

/-- Cloudflare's default provider. -/
def default : Provider := ceramic

/-- Parsing a provider's own name gives that provider back. -/
theorem parse_name (p : Provider) : parse p.name = some p := by
  cases p <;> decide

/-- Parsing only succeeds on a provider's exact wire name. -/
theorem parse_sound {s : String} {p : Provider} (h : parse s = some p) : p.name = s := by
  unfold parse at h
  split at h
  · cases h; simp [name, *]
  · split at h
    · cases h; simp [name, *]
    · split at h
      · cases h; simp [name, *]
      · cases h

/-- `parse` and `name` are inverse: a string parses to `p` exactly when it is `p`'s name. -/
theorem parse_eq_some_iff (s : String) (p : Provider) : parse s = some p ↔ p.name = s :=
  ⟨parse_sound, fun h => h ▸ parse_name p⟩

/-- Distinct providers have distinct wire names. -/
theorem name_injective {p q : Provider} (h : p.name = q.name) : p = q := by
  have hp := parse_name p
  rw [h, parse_name q] at hp
  exact (Option.some.inj hp).symm

/-- `all` lists every provider. -/
theorem mem_all (p : Provider) : p ∈ all := by
  cases p <;> simp [all]

/-- There are exactly three providers, with no duplicates. -/
theorem all_length : all.length = 3 := rfl

theorem all_nodup : all.Nodup := by decide

/-- Cloudflare's default is Ceramic. -/
theorem default_eq_ceramic : default = ceramic := rfl

/-- Names that are not providers are rejected. -/
example : parse "google" = none := by decide
example : parse "" = none := by decide
example : parse "Exa" = none := by decide

end Provider
end WsSpec
