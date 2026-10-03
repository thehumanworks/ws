import WsSpec.Precedence

/-! Backend choice is independent of Cloudflare's provider and credentials. -/
namespace WsSpec

inductive Backend where
  | lightpanda
  | cloudflare
  deriving DecidableEq, Repr

def Backend.name : Backend → String
  | .lightpanda => "lightpanda"
  | .cloudflare => "cloudflare"

def Backend.all : List Backend := [.lightpanda, .cloudflare]

def Backend.platformDefault (bundled : Bool) : Backend :=
  if bundled then .lightpanda else .cloudflare

theorem bundled_backend_default : Backend.platformDefault true = .lightpanda := rfl
theorem unsupported_backend_default : Backend.platformDefault false = .cloudflare := rfl

theorem backend_every_layer_selectable (b default : Backend) :
    (resolve (some b) none none default).1 = b ∧
    (resolve none (some b) none default).1 = b ∧
    (resolve none none (some b) default).1 = b := ⟨rfl, rfl, rfl⟩

end WsSpec
