# Several trusted OIDC issuers, routed by `iss`, with per-issuer claim paths

Status: accepted (2026-10-02)

Relates to: ADR 0069 (auth layer and the `cli-framework-oidc` companion crate),
ADR 0070 (JWKS refetch single-flight and amplification defense), ADR 0071
(standalone `OidcValidator`).

## Context

`OidcValidator` (feature `server`) trusted exactly one issuer. Its config named
one `issuer_url`, one audience policy and one JWKS source, and one JWKS cache
backed it. Services increasingly need to accept tokens from more than one
issuer at once: a user-facing identity provider next to a workload or
machine-identity issuer, two tenants with separate realms, or an old and a new
provider during a migration. Running one validator per issuer and trying each
in turn is wrong in two ways: every token costs a verification attempt (and
possibly a JWKS refetch) against issuers it does not claim, and the error a
caller sees is whichever attempt failed last.

Roles were also hard-coded to `claims["realm_access"]["roles"]`, the place a
Keycloak access token keeps realm roles. Other providers, and Keycloak clients
with custom mappers, put roles somewhere else, and many put group membership in
a claim of its own, often under a namespaced key that contains dots
(`https://example.com/groups`). Consumers worked around this by reading
`OidcClaims::raw` themselves, each with its own path rules.

Downstream tests had a matching gap: the `test-support` helpers synthesized one
issuer served by a mock HTTP server. Testing a two-issuer setup meant two mock
servers and hand-written glue.

## Decisions

**D1 — A list of `OidcValidationConfig`, one per trusted issuer.**
`OidcValidator::new_multi(impl IntoIterator<Item = OidcValidationConfig>)` and
`oidc_validation_layer_multi(..)` take the existing per-issuer config type, so
everything an issuer can be configured with today (audience, JWKS URI or
discovery, algorithms, TTL, clock skew, refetch interval) is per issuer by
construction. No new wrapper type is introduced. `OidcValidator::new` and
`oidc_validation_layer` keep their exact behaviour; `new_multi` with a single
config differs from `new` only in rejecting a foreign `iss` as `UnknownIssuer`
instead of `InvalidIssuer` (D2). `OidcValidator::layer()` turns any validator
into the tower layer, so a service that also calls `validate` directly shares
one set of caches with its middleware.

**D2 — Route by the unverified `iss`, then verify everything against that
issuer.** The token's `iss` is read from the payload *before* any verification,
only to choose which issuer's config applies. The chosen issuer then performs
the full check it always did: algorithm allow-list, key lookup by `kid` in its
own key set, signature, `iss`, `aud` and `exp`. The unverified read
decides nothing on its own: a forged `iss` can only select an issuer whose
keys must then verify the signature. Matching is exact against the normalized
configured issuer, the same comparison the signature-checked `iss` validation
applies, so routing never selects an issuer whose own check would then refuse
the token for a near-miss spelling.

**D3 — An unknown issuer is rejected before any network activity.** An `iss`
that matches no configured issuer (or is missing, or is not a string) is
rejected with the new `TokenRejection::UnknownIssuer` (wire
`error_description="unknown_issuer"`) without discovery, JWKS fetch or key
lookup. Without this, an attacker could pick any issuer in the list for a
forged token and spend that issuer's refetch budget; with it, the
amplification bound of ADR 0070 holds per issuer and an untrusted `iss` costs
nothing. A payload that is not base64url JSON is `Malformed`.

**D4 — Each issuer owns its JWKS cache, discovery state, rate limit and
single-flight gate.** Keys fetched for issuer A are only ever consulted for
tokens routed to A, so a `kid` published by issuer B never validates a token
claiming A, even if both issuers publish the same `kid` string. One issuer's
forced refetches never consume another's `min_refetch_interval`, and a `503`
for unavailable keys advertises the `Retry-After` of the issuer that was
selected. Issuers share one HTTP client.

**D5 — Duplicate issuers are a construction error.** Two configs whose
issuers normalize to the same string would make routing ambiguous; `new_multi`
returns `OidcConfigError::DuplicateIssuer`. An empty list is
`OidcConfigError::MissingField("issuers")`.

**D6 — Roles and groups come from configurable claim paths.** Each issuer has
`roles_claim_path` (default `realm_access.roles`, which preserves the previous
behaviour) and an optional `groups_claim_path` (default none) that fills the
new `OidcClaims::groups`. A path is dot-separated object keys descending
through nested JSON objects; `\.` is a literal dot inside a key and `\\` a
literal backslash. Arrays are never descended into. The value found is read
as a list of strings: string elements of an array, or a single string as a
one-element list; other values, and a path that does not resolve, give an
empty list. Missing roles or groups are not an error: authorization decisions
stay with the consumer, which sees an empty list. A malformed path (empty key,
unknown escape, trailing backslash) is `OidcConfigError::InvalidClaimPath` at
construction, not a per-request surprise.

The browser layers (`oidc_browser_session_layer`, `oidc_dual_mode_layer`) use
their own config type and are unchanged in shape: they read roles from the
default path through the same helper and leave `groups` empty.

**D7 — Keys may be given inline.** `OidcValidationConfig::static_jwks:
Option<JwkSet>` supplies an issuer's keys directly; such an issuer never
performs discovery or a JWKS fetch, and an unknown `kid` is `UnknownKey`.
Setting both `static_jwks` and `jwks_uri`, an empty set, or a symmetric (`oct`)
key is `OidcConfigError::InvalidJwks`. This exists so tests (and issuers whose
keys are distributed out of band) need no HTTP server. `JwkSet` is re-exported
from `jsonwebtoken`.

**D8 — `test_support::TestIssuer` synthesizes issuers without a network.**
`TestIssuer::new(url)` holds a fresh ES256 key (with a `kid` unique to that
issuer), exposes its JWK and JWK set, builds an `OidcValidationConfig` with the
keys inline, and `mint(claims)` signs a token filling `iss`, `iat` and `exp`
when absent. Any number of issuers can be combined in one validator in a plain
unit test. The existing helpers are unchanged.

## Consequences

**Positive**

- A service trusts several issuers with one validator and one layer, each
  issuer fully independent in policy, keys and claim layout, and handlers tell
  them apart by `OidcClaims::iss`.
- An untrusted `iss` is refused cheaply and distinctly, and adding issuers does
  not weaken the per-issuer refetch bounds of ADR 0070.
- Roles and groups no longer require reading `raw` by hand, and namespaced
  keys with dots are expressible.
- Downstream crates can test multi-issuer setups with no mock server.

**Negative / costs**

- `OidcValidationConfig` gains three public fields and `OidcClaims` gains
  `groups`. Code that builds either with a struct literal listing every field
  must add the new fields or switch to `..OidcValidationConfig::new(..)`;
  construction through `OidcValidationConfig::new` and field assignment is
  unaffected.
- `TokenRejection` gains `UnknownIssuer`. It is `#[non_exhaustive]`, so
  matches already carry a wildcard arm.
- The default roles path now also accepts a single string where it previously
  accepted only an array; a token with a non-array `realm_access.roles` string
  yields one role instead of none.
- Routing relies on the token's `iss` matching the normalized configured
  issuer exactly. An issuer that emits a trailing slash or other
  non-normalized spelling cannot be trusted by either `new` or `new_multi`;
  that limitation predates this ADR.

## Alternatives considered

1. **Try every issuer in turn.** No routing step, but each token is verified
   against issuers it does not claim, an unknown-`kid` refetch can be
   triggered on every issuer for one forged token, and the reported rejection
   is arbitrary. Rejected.
2. **Route by `kid` instead of `iss`.** `kid` values are chosen independently
   by each issuer and can collide; routing by them would let one issuer's key
   set answer for another's tokens. Rejected (D4).
3. **A JSONPath or JSON Pointer expression for claim paths.** More expressive
   than needed (roles and groups are object lookups in every provider we
   considered), and JSON Pointer's `/` separator collides with URL-shaped claim
   names, which are the common case for namespaced claims. A dot path is the
   `claim_path` convention the config service's assignment rules already use
   (object keys only, unresolved means nothing), extended here with one escape
   for keys that contain dots. Rejected.
