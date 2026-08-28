# ADR 0072: Managed configuration — enforced policy is a final veto, not a layer

- Date: 2026-08-28
- Status: Accepted
- Relates to: ADR 0067 (first-class layered configuration), ADR 0065 (strict by default); specs 016, 021, 022, 023

## Context

ADR 0067 established a layered resolver with a single precedence chain:

```
defaults → config file(s) → environment → CLI global flags → builder overrides
```

Enterprise deployments need a *central* authority over that chain. An
organisation running a fleet of desktops, mobile apps, and services must be able
to say "the bridge listener is off on every machine, and no local action can turn
it on." Two questions follow: where does centrally-authored configuration sit in
the precedence chain, and can the local user override it?

Answering "it is just another layer" is the obvious move and it is wrong in both
directions. Placed *below* environment and flags, an organisation's mandate is
defeated by one environment variable — management becomes advisory, which is
exactly what a tight environment cannot accept. Placed *above* everything as a
single opaque layer, an organisation can no longer express a *default* that a
user may reasonably adjust (screen layout, workspace choice), so every managed
app becomes maximally rigid and organisations stop deploying management at all.

Windows Group Policy and Chrome Enterprise both resolved this the same way, and
both are the prior art an enterprise administrator will expect us to match.

## Decision

Centrally-authored configuration arrives as **two distinct trees** in one
**Policy** document, and they occupy **two different positions** in the
resolution order:

```
defaults → recommended → config file → environment → flags → builder overrides → ENFORCED
```

- **Recommended** sits directly above built-in defaults. It answers "what should
  this setting be if nobody says otherwise" — every local mechanism still
  overrides it.
- **Enforced** is applied **last, as a veto over the fully resolved value** —
  above environment variables, above CLI flags, above programmatic builder
  overrides. There is no local mechanism that outranks it.

Enforcement is a property of the field, declared per-field by the organisation
in the Policy document, not a global mode. The same deployment can enforce two
fields and merely recommend five others.

The resolver additionally exposes **Provenance** for every field: the resolved
value plus which layer produced it and whether it is locked. This is not a
debugging nicety — a settings UI that cannot distinguish "enforced by your
organisation" from "you chose this" will silently discard user edits, and a
support engineer cannot otherwise answer "why is this machine behaving this way".

## Consequences

- `OTEL_SDK_DISABLED`-style "final veto" handling (ADR 0068) is no longer a
  one-off special case; enforcement is the general mechanism and that veto is one
  instance of the shape.
- Every consumer of `ctx.config()` is unaffected: precedence is resolved before
  the value is handed out. Only surfaces that *display or edit* configuration
  need the Provenance API.
- A field can be made enforceable-but-unenforced. Organisations tighten by
  editing policy, not by shipping a new client build — which is the whole point.
- Local `config file` keeps ADR 0065's strict deny-unknown-fields posture, but
  the two server-authored trees deliberately **warn-and-ignore** unknown or
  type-mismatched keys instead. A server is routinely newer than a deployed
  client; hard-failing there would mean an organisation adopting one new setting
  bricks every client that has not yet updated. Strictness protects the file a
  human hand-edits; tolerance protects the fleet from version skew.
- Enforcement is **anti-footgun, not anti-malice**. The client runs as the user
  and the local policy cache is user-writable, so a determined local
  administrator can bypass it. Cryptographic signing of policy documents is
  deliberately deferred to a later spec; until it lands, no security claim
  stronger than "prevents accidental and casual divergence" may be made in
  user-facing documentation.
