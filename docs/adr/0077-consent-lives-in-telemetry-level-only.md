# ADR 0077: Consent is one field, `telemetry.level`, recommendable but never enforceable, and it never roams

- Date: 2026-09-04
- Status: Accepted
- Relates to: ADR 0072 (enforced veto), ADR 0073 (manifest flags, `enforceable`), ADR 0076 (probe ids), ADR 0079 (metric identity); specs 021, 025

## Context

Telemetry on an end-user **Deployment** requires the person's **Consent**. The
framework must store that choice somewhere it can be resolved alongside
organisation Policy, shown in a settings UI, and reset. Three shapes were
weighed.

1. **A boolean plus a level** — `telemetry.enabled` and `telemetry.level`.
   Two fields that must agree; "enabled but level off" and "disabled but level
   debug" are both representable and both meaningless.
2. **A consent record** — level, timestamp, notice version, text hash. Closer
   to what a lawyer would draw, but the **Telemetry notice** informs and never
   asks, so there is no acceptance event to record; the record would document a
   ceremony that does not happen.
3. **The level itself is the consent.** One field; `off` is "no consent".

Independently, the field's **Scope** had to be chosen. `user` scope would let
consent follow the person across devices via roaming user configuration.
`machine` scope keeps it on one **Install**.

## Decision

**Consent is `telemetry.level` and nothing else.** Its manifest flags are:

| Flag | Value | Why |
|---|---|---|
| `scope` | `machine` | one Install, see below |
| `manageable` | `true` | an organisation may recommend a level |
| `enforceable` | `false` | an organisation may never grant consent on a person's behalf |
| `local_only` | `false` | `local_only` fields are dropped from *both* server trees, which would block the recommendation too |
| `restart_required` | `true` | phase 1 resolves once per process |

Resolution follows the ordinary layer order (ADR 0072), then on an end-user
Deployment the effective level is **clamped** to what the layers below
environment, flags and builder overrides would have produced. A builder
override or an environment variable may lower the level; only the person, or a
Policy the person has not overridden, may raise it. Kill switches
(`<APP>_TELEMETRY_DISABLED`, `OTEL_SDK_DISABLED`, `DO_NOT_TRACK`) sit above
everything and force `off`. A **service** Deployment has no Consent: its level
is configured by its operator.

### Consent is per Install and never roams

`telemetry.level`, `attribution`, `install_id`, `notice_shown` and every
`<probe>.enabled` switch are `machine` scope. Nothing under `telemetry.` has
`user` scope, so nothing under it is eligible for roaming. The reasons, in
order of weight:

- **Consent and the pseudonymous id must share a boundary.** Support correlates
  to one Install. Consent that roamed while the id did not would have a second
  device exporting under an id support had never seen, authorised by a choice
  made elsewhere.
- **The Telemetry notice contract only holds locally.** A roamed value lands in
  the config file, which is exactly the layer the notice treats as "the person
  already chose"; the second device would go silent.
- **`telemetry reset` means "fresh install".** It cannot mean that if the next
  sync brings the old level back.
- **It is what `machine` scope already means in practice.** The config file
  lives under the user's profile on one device, so a machine-scope value has
  always been per account per device — one Install.
- **It is the reversible direction.** Flipping one field to `user` scope later
  is a manifest edit and a notice rule; starting with roaming and pulling it
  back leaves ids already copied across devices.

The accepted cost: a person with three laptops opts in three times.

## Consequences

- The framework **inserts a `telemetry` section into the application's
  published Config manifest** at build time, so the config service can validate
  a Policy that mentions it and an administrator's editor can offer it. An app
  that declares its own top-level `telemetry` key fails to build. Both ends
  reject an Enforced `telemetry.level`: the service at policy write time, the
  client resolver at resolution.
- An organisation can recommend a level only after the application has
  republished its manifest from a build that inserts the section (cli-framework
  ≥ 0.6.0). Until then the key is unknown server-side and the write is rejected.
- The roaming upload filter (`filter_user_scoped`) selects on `scope` alone
  today. Spec 025 tightens it to also exclude `local_only` and `secret`
  fields, which the manifest documentation already promises never travel that
  channel. That is a defence in depth; the scope choice above is what keeps
  consent local.
- `telemetry reset` deletes the framework-owned telemetry file: level, id,
  notice marker and probe switches go together.
- There is no `telemetry.enabled`. Documentation, `doctor` and the notice all
  speak in levels.
