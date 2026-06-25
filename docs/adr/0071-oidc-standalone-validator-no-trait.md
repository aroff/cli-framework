# ADR 0071: Standalone OIDC validation — concrete `OidcValidator`, no `TokenValidator` trait

- Date: 2026-06-25
- Status: Accepted
- Relates to: ADR 0069 (auth layer / `cli-framework-oidc` server feature), ADR 0070 (JWKS single-flight); spec 018

## Context

The `server` feature exposed JWT validation only as a tower `Layer`
(`oidc_validation_layer`). Library consumers that mount EntityStore's router in-process
need to verify a single token string and get back typed claims or a typed error, behind
their **own** trait — without standing up an axum request. Spec 018 adds a callable
`OidcValidator::validate(token) -> Result<OidcClaims, OidcValidationError>`.

The open question was whether this crate should also define a `TokenValidator` *trait* —
the structural dual of the client-side `TokenProvider` trait that `cli-framework`'s `auth`
feature already owns (client acquires a token; server verifies one).

## Decision

`cli-framework-oidc` exposes only the **concrete** `OidcValidator` with inherent
`validate` / `validate_authorization` methods. It does **not** define a `TokenValidator`
trait. Consumers that want a trait seam (e.g. EntityStore's `entitystore-auth-oidc`)
define it themselves and implement it by calling `OidcValidator::validate`.

The decisive reason is **directionality of the dependency**, not IdP-neutrality.
`TokenProvider` lives in the framework because the framework runtime *calls* it (the
401-retry path, `AuthenticatedHttpClient`, auto-registered `auth` commands — ADR 0069's
"a hook the runtime actually exercises"). Nothing in the framework runtime ever calls a
token *validator*: the consumer's own router does, behind the consumer's own application
trait. A trait with zero framework callers is precisely the inert surface ADR 0060 removed
and ADR 0069 was careful not to reintroduce. Hosting it here would be a trait this crate
never invokes.

## Typed rejection (companion decision)

The standalone error is **fully typed**: `OidcValidationError::InvalidToken` carries a
`TokenRejection` enum (`Undecodable`, `UnsupportedAlgorithm`, `UnknownKey`, `Malformed`,
`Expired`, `NotYetValid`, `InvalidSignature`, `InvalidIssuer`, `InvalidAudience`) rather
than the `error_description: Option<String>` the first draft proposed. The whole point of a
callable validator is that a consumer maps outcomes onto its own error model — a magic-string
payload would force string-matching and is not a typed API. The wire-facing
`error_description` strings are *derived* from the variant in the single `error_to_response`
mapping, so HTTP responses stay byte-identical and the closed reason set is preserved. This
also forced an explicit distinction the string model blurred: an *undecodable* header emits a
bare `error="invalid_token"` (no description), whereas a *malformed-claims* token emits
`error_description="malformed_token"`.

## Consequences

- Multiple consumers each define their own validator trait. The duplication is bounded by
  having the first consumer (EntityStore) export its trait to its own sub-consumers, not by
  this crate hosting an abstraction it never uses.
- Should the framework runtime ever gain a server-side hook that itself needs to call a
  validator (symmetric to the `auth(BoxCloneLayer)` mount), this decision should be
  revisited — at that point a framework-owned `TokenValidator` would earn its place by the
  same test that justified `TokenProvider`.
