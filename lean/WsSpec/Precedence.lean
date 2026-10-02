import WsSpec.Provider

/-!
# Configuration precedence

`ws` resolves the provider and the gateway from four layers:
command-line flag, environment variable, config file, built-in default.
`resolve` is the whole rule; `ws::config::pick` in `src/config.rs` implements it.
-/

namespace WsSpec

/-- Which layer supplied a value. -/
inductive Source where
  | flag
  | env
  | file
  | default
  deriving DecidableEq, Repr

/-- The precedence rule: flag, then environment, then file, then default. -/
def resolve {α : Type} (flag env file : Option α) (default : α) : α × Source :=
  match flag, env, file with
  | some v, _, _ => (v, .flag)
  | none, some v, _ => (v, .env)
  | none, none, some v => (v, .file)
  | none, none, none => (default, .default)

variable {α : Type}

/-- A flag always wins. -/
theorem flag_wins (v : α) (env file : Option α) (d : α) :
    resolve (some v) env file d = (v, .flag) := rfl

/-- Without a flag, the environment wins over the file and the default. -/
theorem env_wins (v : α) (file : Option α) (d : α) :
    resolve none (some v) file d = (v, .env) := rfl

/-- Without a flag or environment value, the file wins over the default. -/
theorem file_wins (v d : α) : resolve none none (some v) d = (v, .file) := rfl

/-- The default applies only when every layer is silent. -/
theorem default_last (d : α) : resolve (none : Option α) none none d = (d, .default) := rfl

theorem default_iff (flag env file : Option α) (d : α) :
    (resolve flag env file d).2 = .default ↔ flag = none ∧ env = none ∧ file = none := by
  cases flag <;> cases env <;> cases file <;> simp [resolve]

/-- The rule is the usual "first one that is set": it agrees with `<|>` chaining. -/
theorem resolve_eq_orElse (flag env file : Option α) (d : α) :
    (resolve flag env file d).1 = (flag <|> env <|> file).getD d := by
  cases flag <;> cases env <;> cases file <;> rfl

/-- The result is never invented: it is one of the four inputs, and the
reported source is the layer it actually came from. -/
theorem resolve_faithful (flag env file : Option α) (d : α) :
    match (resolve flag env file d).2 with
    | .flag => flag = some (resolve flag env file d).1
    | .env => flag = none ∧ env = some (resolve flag env file d).1
    | .file => flag = none ∧ env = none ∧ file = some (resolve flag env file d).1
    | .default => flag = none ∧ env = none ∧ file = none ∧ (resolve flag env file d).1 = d := by
  cases flag <;> cases env <;> cases file <;> simp [resolve]

/-- Lower layers cannot affect the outcome once a higher layer is set. -/
theorem lower_layers_irrelevant (v : α) (env env' file file' : Option α) (d d' : α) :
    resolve (some v) env file d = resolve (some v) env' file' d' := rfl

/-- With nothing configured, the provider is Cloudflare's default, Ceramic. -/
theorem provider_default :
    (resolve (none : Option Provider) none none Provider.default).1 = Provider.ceramic := rfl

/-- The environment overrides a configured default provider (the user-facing guarantee). -/
theorem env_overrides_configured_default (envP fileP : Provider) :
    (resolve none (some envP) (some fileP) Provider.default).1 = envP := rfl

/-- Any provider is reachable from any single layer, so users can always choose one. -/
theorem every_provider_selectable (p : Provider) :
    (resolve (some p) none none Provider.default).1 = p ∧
    (resolve none (some p) none Provider.default).1 = p ∧
    (resolve none none (some p) Provider.default).1 = p := ⟨rfl, rfl, rfl⟩

end WsSpec
