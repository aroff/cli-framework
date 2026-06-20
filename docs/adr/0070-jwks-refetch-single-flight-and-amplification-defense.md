# ADR 0070: JWKS refetch — single-flight coalescing and forged-`kid` amplification defense

- Date: 2026-06-20
- Status: Accepted
- Relates to: ADR 0069 (auth layer / `cli-framework-oidc` server feature)

## Context

The `server` feature of `cli-framework-oidc` validates incoming JWTs in
`oidc_validation_layer`. To verify a token's RS256 signature it needs the issuer's
**public** key, distributed by the IdP (Keycloak) as a **JWKS** (JSON Web Key Set) at
the `jwks_uri`. The server caches that key set (`JwksCache`, TTL default 300s) so the
common path is local CPU work with no network call.

JWKS documents contain multiple keys, each tagged with a `kid` (key ID). A token's
header names the `kid` it was signed with; the server selects that key from the set.
Providers publish several keys at once and rotate them periodically — this is how they
roll signing keys without downtime.

### The rotation problem

Immediately after Keycloak rotates its signing key, new tokens carry `kid=new` while the
server's cache (still inside its TTL) only holds `kid=old`. If the server rejected any
token whose `kid` is absent from the cache, **every request would 401 for up to the full
TTL** after each rotation. To avoid that, the server treats an unknown `kid` on an
otherwise-fresh cache as a rotation signal and forces a JWKS refetch (shipped earlier as
gap "C1").

### The threat this opens

The JWT header — including `kid` — is **not** signature-protected. An attacker can mint a
token with any `kid` value at zero cost, without owning any key. Composed with
unknown-`kid`-triggers-refetch, this is an amplification primitive:

- One forged token (free) → one outbound HTTPS fetch from our service to Keycloak.
- Thousands of concurrent requests, each with a distinct random `kid` → thousands of
  simultaneous outbound fetches (a thundering herd).

This is a **reflected amplification / denial-of-service vector against our own IdP**.
Keycloak is shared platform infrastructure backing every product's realm, so a forged-`kid`
flood against a single service can degrade authentication platform-wide.

Severity is bounded to **availability**: the forged tokens are still ultimately *rejected*
at signature verification. There is no authentication bypass. This ADR addresses resource
exhaustion only.

### Why the existing control is insufficient on its own

A rate-limit already exists: `min_refetch_interval` (default 60s, `last_forced_refetch`
timestamp) caps how *often* a forced refetch may occur. But in the current code the
timestamp is written only *after* the fetch completes. Concurrent requests that arrive
during the fetch all pass the rate-limit gate before it is set, so a burst still launches
N simultaneous fetches. Rate-limiting bounds *frequency over time*; it does not bound
*concurrency at an instant*.

## Decision

Defend the refetch path with **two orthogonal controls**, defense-in-depth:

| Control | Bounds | Mechanism |
| --- | --- | --- |
| **Rate-limit** | Frequency — ≤1 forced refetch per `min_refetch_interval` | `last_forced_refetch` timestamp (already present) |
| **Single-flight** | Concurrency — ≤1 JWKS fetch in flight at any instant | refetch coalescing gate (this ADR) |

### Single-flight coalescing

When multiple requests concurrently discover they need a refetch (TTL-expired or
unknown-`kid`), **exactly one** performs the outbound fetch; the others wait for and
**share** its result, then re-attempt validation against the now-fresh cache.

Implementation shape:

1. A dedicated async refetch gate, **separate** from the `jwks_cache` read-lock, so a slow
   network fetch never blocks validation of tokens whose `kid` is already cached.
2. **Double-checked**: after acquiring the gate, re-read the cache. If a prior holder
   already refreshed it (the needed `kid` is now present, or the cache was refetched after
   this request's initial read), skip the fetch entirely.
3. The rate-limit check and the `last_forced_refetch` write happen **inside** the gate, so
   the frequency bound is enforced atomically rather than racing the fetch.

### Resulting guarantee

Forged-`kid` traffic is bounded on two axes simultaneously: at most one JWKS fetch is in
flight at any moment, and at most one fetch occurs per `min_refetch_interval`. Legitimate
requests arriving during a real key rotation **share** the single fetch and all succeed —
no spurious 401s. Forged tokens are still rejected at signature verification.

## Consequences

**Positive**

- Eliminates the forged-`kid` thundering-herd against Keycloak; outbound fetch load is
  bounded regardless of inbound request concurrency.
- Removes rotation-induced spurious 401s: concurrent legitimate requests coalesce onto one
  fetch instead of racing or being rejected.
- The two controls are independent and each defensible in isolation to a security review.

**Negative / residual risk (disclosed)**

- Requests that need a refetch **wait** on the in-flight fetch rather than failing fast.
  This is a deliberate latency-vs-availability trade: a brief tail latency during rotation
  in exchange for zero rotation-induced 401s. The wait is bounded by the HTTP client
  timeout already configured on the fetch.
- Single-flight bounds concurrency to 1 but not total volume across time; the rate-limit is
  what bounds volume. Both are required — neither replaces the other.

## Alternatives considered

1. **Rate-limit only (no single-flight).** Simpler, but either permits a thundering herd
   within one interval (timestamp set after fetch, as today) or — if the gate is tightened
   to set the timestamp first — rejects concurrent legitimate requests with spurious 401s
   during rotation. Rejected: does not bound concurrency without harming the legitimate
   rotation path.

2. **No unknown-`kid` refetch; rely on TTL alone.** Removes the amplification primitive
   entirely, but reintroduces rotation outages (up to one TTL of 401s after every key
   roll). Lowering the TTL shrinks the outage window at the cost of higher steady-state
   fetch load. Rejected: trades a security-availability concern for a baseline-availability
   regression.

3. **Pre-validate `kid` shape / allowlist before refetch.** The server cannot know which
   `kid` values are legitimate without the keys it is trying to fetch, so any such filter
   is guesswork and brittle across rotations. Rejected as ineffective.

The chosen composition (refetch-on-unknown-`kid` + rate-limit + single-flight) matches the
established pattern in mature JWKS-caching libraries.
