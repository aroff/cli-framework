# ADR 0073: The Config manifest is a canonical document, derived from code for Rust apps

- Date: 2026-08-28
- Status: Accepted
- Relates to: ADR 0072 (enforced veto), ADR 0064 (derive `CommandSpec`), ADR 0065 (strict by default); specs 021, 022

## Context

Managed configuration needs machine-readable knowledge of an application's
configuration surface, for four different consumers:

1. A **settings renderer** that builds an editing UI dynamically — AI Desktop's
   settings window, an EntityAI web app, a mobile settings screen.
2. An **administrator's policy editor**, which must offer the organisation the
   set of fields it may enforce or recommend, with types and constraints.
3. The **config service**, which validates that a submitted policy only names
   fields that exist, of the right type, that the organisation is allowed to set.
4. The **client resolver**, which needs per-field flags to decide where a value
   may live and who may write it.

Without such a declaration each of those consumers hard-codes its own copy of
the app's field list, and the copies drift. The reference points are Group
Policy's ADMX templates and Chrome's policy templates: both ship a declaration of
the manageable surface separately from the values.

Two axes were open. First, **who authors it**: a hand-written document, or
something derived from the Rust config struct. Deriving guarantees code and
declaration cannot drift and matches ADR 0064's precedent, but non-Rust
applications (EntityAI apps, iOS, Android, web) have no Rust struct to derive
from — and those are first-class consumers, not an afterthought. Second, **what
format**: JSON Schema with vendor extensions, or a purpose-built schema. JSON
Schema brings off-the-shelf validators, but its full expressiveness (`oneOf`,
conditional subschemas) is far more than a renderer on a phone wants to
implement, and it has no natural place for the policy metadata that is the
entire reason we need this.

## Decision

The **Config manifest** is a **JSON document with a purpose-built schema**, and
that document is **canonical**. For Rust applications a derive macro generates it
from the config struct at compile time, embeds it in the binary, and exposes it
via a `config manifest` command; non-Rust applications author the same document
by hand. Every consumer — renderer, admin tool, service, resolver — reads only
the document. The derive macro is a convenience for one language, never a
privileged path.

Each field declares its type, default, human label and description, grouping and
ordering for renderers, constraints, and a fixed set of policy flags:

| Flag | Meaning |
|---|---|
| `scope` | `machine` (this installation), `user` (roams across the person's devices), or `org` (one value organisation-wide, delivered only via Policy). Orthogonal to Enforced/Recommended for `machine` and `user` fields — an organisation may recommend *or* enforce either. `org` is the exception: it has no local existence to recommend a default over, so it is always delivered as Enforced. |
| `platforms` | Which platforms the field applies to; absent means all |
| `secret` | Never stored in a config file or a Policy; lives in a secret store |
| `local_only` | Server layers may never set it (bootstrap settings such as the service URL) |
| `protected` | Writable only by the application's own privileged surface, never through the application's own **command or mutation surface** (an in-process agent, an external driver issuing commands over some channel). Governs *that* surface only — it says nothing about Policy authority. An organisation may still `recommend` or `enforce` a `protected` field through the ordinary Policy channel; `local_only` is the flag that excludes a field from Policy entirely. The two commonly pair (a field an organisation must never touch at all is both), but a field that an organisation *should* be able to lock down while no local command may touch it — a device's own command-and-control surface being remotely disabled being the canonical case — is `protected` without `local_only`. |
| `manageable` | `false` means an organisation may not set it even as a recommendation |
| `enforceable` | Defaults `true`. `false` means an organisation may recommend a default but may never force it — a policy placing such a field in Enforced is invalid, given the same tolerant drop-and-warn treatment as any other misplaced field. For a field whose "on" state is itself an act of standing consent by the person using the device, forcing it would mean the organisation granting consent on that person's behalf, which no Recommended/Enforced pair can be allowed to do silently |
| `restart_required` | Renderer badge; excluded from hot-apply |

One application id covers all of a product's form factors; `platforms` handles
per-platform fields. This is what makes "my settings follow me from the desktop
app to the mobile app" resolvable — a shared identity plus a shared namespace.

A one-way export to JSON Schema is provided for ecosystems that want generic
validation; nothing in the system consumes it.

## Consequences

- Field-level policy rules stop being scattered ad-hoc lists (a denylist here, a
  hard-coded "substrate fields" set there) and become one declared property,
  enforced identically by the client resolver and by server-side policy
  validation.
- The manifest is a **public contract**. Removing a field or tightening its type
  is a breaking change for stored policies and roaming user config; schema
  versioning and migrations (spec 016) cover the value side, and the service
  validates policies against the manifest at load so a break is loud and central
  rather than silent and per-device.
- Version skew is expected and tolerated by construction: a client always
  validates and renders against **its own embedded manifest**, never the server's
  copy. The server's copy exists for server-side validation and administrator
  tooling. ADR 0072's warn-and-ignore rule for unknown policy keys is what
  absorbs the difference.
- A reference React renderer is deliberately **not** built here. It is a
  UI-library concern and will live in the EntityAI UI-kit repository; this crate
  ships the contract and the Rust-side tooling only.
