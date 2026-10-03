/-!
# Page fetching

The local decisions `ws fetch` makes before and while it talks to a page's
host: which URLs it accepts, which addresses it will connect to, and how a
`Content-Type` is treated. Mirrors `validate::url`, `validate::ipv4_is_public`
and `fetch::kind_of` in the Rust code.

Two deliberate differences from Rust, both outside the generated vectors: Rust
rejects every Unicode whitespace character where the model knows only the
space and the control characters, and Rust lower-cases and strips parameters
from a `Content-Type` before classifying it, which the model takes as done.
-/

namespace WsSpec

/-! ## URLs -/

/-- Longest URL accepted, in characters. -/
def urlMaxChars : Nat := 2048

/-- URL lengths accepted. -/
def urlLenValid (n : Nat) : Bool := n ≤ urlMaxChars

/-- Control characters: C0, DEL and C1, as Rust's `char::is_control`. -/
def isControl (c : Char) : Bool := c.val < 0x20 || (0x7F ≤ c.val && c.val ≤ 0x9F)

/-- Characters a URL may not contain. -/
def isBlank (c : Char) : Bool := c == ' ' || isControl c

/-- What follows an `http://` or `https://` scheme (matched in any ASCII case). -/
def schemeRest (cs : List Char) : Option (List Char) :=
  if (cs.take 7).map Char.toLower = "http://".toList then some (cs.drop 7)
  else if (cs.take 8).map Char.toLower = "https://".toList then some (cs.drop 8)
  else none

/-- A host must start right after the scheme. -/
def hostStartOk : List Char → Bool
  | [] => false
  | c :: _ => !(c == '/' || c == '?' || c == '#' || c == ':' || c == '@')

/-- The URL check on a list of characters. -/
def urlCharsValid (cs : List Char) : Bool :=
  !cs.any isBlank && urlLenValid cs.length &&
    match schemeRest cs with
    | some rest => hostStartOk rest
    | none => false

/-- A URL `ws fetch` will request. -/
def urlValid (s : String) : Bool := urlCharsValid s.toList

theorem urlLenValid_iff (n : Nat) : urlLenValid n = true ↔ n ≤ 2048 := by
  unfold urlLenValid urlMaxChars
  exact decide_eq_true_iff

/-- An accepted URL is within the length bound. -/
theorem urlCharsValid_length {cs : List Char} (h : urlCharsValid cs = true) :
    cs.length ≤ 2048 := by
  simp [urlCharsValid, urlLenValid, urlMaxChars] at h
  exact of_decide_eq_true h.1.2

/-- An accepted URL contains no space and no control character, so it cannot
smuggle a second header line into the request. -/
theorem urlCharsValid_no_blank {cs : List Char} (h : urlCharsValid cs = true) :
    ∀ c ∈ cs, isBlank c = false := by
  simp [urlCharsValid] at h
  intro c hc
  exact h.1.1 c hc

/-- An accepted URL has an `http` or `https` scheme followed by a host. -/
theorem urlCharsValid_scheme {cs : List Char} (h : urlCharsValid cs = true) :
    ∃ rest, schemeRest cs = some rest ∧ rest ≠ [] := by
  simp only [urlCharsValid, Bool.and_eq_true] at h
  cases hs : schemeRest cs with
  | none => simp [hs] at h
  | some rest =>
    refine ⟨rest, rfl, ?_⟩
    intro hr
    simp [hs, hr, hostStartOk] at h

/-- Anything without an `http(s)` scheme is refused, whatever else it holds. -/
theorem urlCharsValid_needs_scheme {cs : List Char} (h : schemeRest cs = none) :
    urlCharsValid cs = false := by
  simp [urlCharsValid, h]

/-- Boundary behaviour, as tested in Rust. -/
example : urlLenValid 2048 = true := by decide
example : urlLenValid 2049 = false := by decide
example : isBlank ' ' = true := by decide
example : isBlank '\n' = true := by decide
example : isBlank '\r' = true := by decide
example : isBlank 'a' = false := by decide
example : hostStartOk [] = false := by decide
example : hostStartOk ['/'] = false := by decide
example : hostStartOk ['e'] = true := by decide

/-! ## Addresses -/

/-- Whether the IPv4 address `a.b.c.d` is on the public internet. -/
def ipv4Public (a b c _d : Nat) : Bool :=
  !(a == 0 || a == 10 || a == 127
    || (a == 100 && 64 ≤ b && b ≤ 127)
    || (a == 169 && b == 254)
    || (a == 172 && 16 ≤ b && b ≤ 31)
    || (a == 192 && b == 168)
    || (a == 192 && b == 0 && c == 0)
    || (a == 198 && (b == 18 || b == 19))
    || 224 ≤ a)

/-- Loopback is never public. -/
theorem loopback_not_public (b c d : Nat) : ipv4Public 127 b c d = false := by
  simp [ipv4Public]

/-- The RFC 1918 private ranges are never public. -/
theorem private10_not_public (b c d : Nat) : ipv4Public 10 b c d = false := by
  simp [ipv4Public]

theorem private172_not_public (b c d : Nat) (h : 16 ≤ b ∧ b ≤ 31) :
    ipv4Public 172 b c d = false := by
  simp [ipv4Public, h.1, h.2]

theorem private192_not_public (c d : Nat) : ipv4Public 192 168 c d = false := by
  simp [ipv4Public]

/-- Link-local addresses, where cloud metadata services live, are never public. -/
theorem linkLocal_not_public (c d : Nat) : ipv4Public 169 254 c d = false := by
  simp [ipv4Public]

/-- "This network" and everything from multicast upwards are never public. -/
theorem zero_not_public (b c d : Nat) : ipv4Public 0 b c d = false := by
  simp [ipv4Public]

theorem multicast_and_above_not_public (a b c d : Nat) (h : 224 ≤ a) :
    ipv4Public a b c d = false := by
  simp [ipv4Public, h]

/-- The decision never depends on the last octet. -/
theorem ipv4Public_last_octet (a b c d e : Nat) : ipv4Public a b c d = ipv4Public a b c e := rfl

example : ipv4Public 169 254 169 254 = false := by decide
example : ipv4Public 8 8 8 8 = true := by decide
example : ipv4Public 172 15 0 1 = true := by decide
example : ipv4Public 172 32 0 1 = true := by decide
example : ipv4Public 223 255 255 255 = true := by decide

/-! ## Content types -/

/-- How a page body is treated. -/
inductive Kind where
  | html
  | markdown
  | text
  deriving DecidableEq, Repr

namespace Kind

/-- The name used in the vectors. -/
def name : Kind → String
  | html => "html"
  | markdown => "markdown"
  | text => "text"

end Kind

/-- Classifies a media type (lower case, parameters removed). `none` means the
content is not text and is refused. -/
def kindOf (mime : String) : Option Kind :=
  if mime = "text/html" ∨ mime = "application/xhtml+xml" then some .html
  else if mime = "text/markdown" ∨ mime = "text/x-markdown" then some .markdown
  else if mime = "application/json" ∨ mime = "application/xml"
      ∨ mime = "application/javascript" then some .text
  else if mime.startsWith "text/" || mime.endsWith "+json" || mime.endsWith "+xml" then
    some .text
  else none

/-- Only the two HTML media types are converted to Markdown. -/
theorem kindOf_html_iff (mime : String) :
    kindOf mime = some .html ↔ mime = "text/html" ∨ mime = "application/xhtml+xml" := by
  unfold kindOf
  constructor
  · intro h
    split at h
    · assumption
    · split at h
      · cases h
      · split at h
        · cases h
        · split at h <;> cases h
  · intro h
    simp [h]

example : kindOf "text/html" = some .html := by decide
example : kindOf "application/xhtml+xml" = some .html := by decide
example : kindOf "text/markdown" = some .markdown := by decide
example : kindOf "application/json" = some .text := by decide

end WsSpec
