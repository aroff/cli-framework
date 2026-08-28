# ADR 0074: Exactly one Profile per identity; inheritance is flattened server-side

- Date: 2026-08-28
- Status: Accepted
- Relates to: ADR 0072 (enforced veto), ADR 0073 (Config manifest); specs 022, 023

## Context

An organisation does not want one configuration for everybody. Developers,
analysts, and kiosk machines need different settings, and those groupings already
exist in the identity provider as Keycloak groups. The question is how a client's
identity turns into the configuration it receives.

Group Policy answers this with objects linked at multiple levels (site, domain,
organisational unit), inherited and merged, with block-inheritance and
enforcement flags to control the resulting precedence. It is powerful and it is
the single most common source of "why is this machine configured this way"
support escalations: answering that question requires replaying a merge over a
tree, per machine.

The temptation to reproduce it is real, because the alternative — one flat
grouping per identity — appears to force every grouping to restate the whole
organisational baseline, which then rots as the baseline evolves.

A second question is whether "team" is a level of its own. It is not: a team is
a *targeting* input (which identities receive which values), not a *scope*
(whose value it is). ADR 0073's `scope` axis already answers ownership.

## Decision

**Exactly one Profile applies to a given (identity, application).** The service
evaluates an ordered list of assignment rules against the validated token claims
and takes the **first match**; an optional default profile catches the remainder;
no match and no default means the application is unmanaged for that identity.
Teams are expressed as claim-matching rules (typically on Keycloak groups), not
as a scope or a level.

To remove the restate-the-baseline problem without reintroducing merge
archaeology, a Profile may declare **a single parent** it inherits from. The
service resolves the chain — parent trees deep-merged beneath the child's, cycles
rejected at load — and serves **one already-flattened Policy document**. Clients
never learn that inheritance exists; the wire contract has no notion of it.

**Multiple inheritance is refused.** One parent keeps the resolved value's origin
answerable by walking a line rather than a lattice.

## Consequences

- "Why is this machine configured this way" is answered by one profile name and
  one matched rule. The service exposes exactly that as a support endpoint,
  returning the profile and the rule that selected it without returning
  configuration values.
- Rule *order* is load-bearing. An identity in several teams gets a deterministic
  answer, but the determinism comes from the administrator's ordering, so the
  ordering is part of the stored assignment document, not an incidental property
  of a map.
- All inheritance complexity is confined to one server-side function with no
  client counterpart. Mobile and web clients stay trivial to implement — a
  significant part of why the contract is worth having.
- Organisations migrating from Group Policy will find capabilities missing
  (block-inheritance, per-link enforcement, WMI filters). That is intentional. If
  a genuine need for multi-profile merging appears, it arrives as a new
  server-side resolution mode behind a new contract version, not as a gradual
  loosening of this one.
