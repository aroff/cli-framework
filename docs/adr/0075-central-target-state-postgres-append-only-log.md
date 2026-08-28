# ADR 0075: Central target state lives in Postgres with an append-only mutation log; no GitOps

- Date: 2026-08-28
- Status: Accepted
- Relates to: ADR 0072 (enforced veto), ADR 0074 (one Profile per identity); specs 022, 023

## Context

The config service needs a system of record for manifests, policies, profile
assignments, and roaming user configuration. Two candidate architectures were
weighed seriously.

**Files in a Git repository.** The service would read a directory that Git
delivers. Change control is a pull request, audit is `git log`, rollback is
`git revert`, and rollout reuses the deployment machinery already operating this
fleet — all for no code written. This was the initial recommendation.

**A database as target state.** The service owns its data and exposes an
administrative API for changes.

The deciding requirement is that organisation administrators must be able to
change individual setting values through an API — ultimately through a settings
UI inside a product, not a code-review workflow. Group Policy has a console and
Chrome Enterprise has an admin panel for the same reason: the people who
administer policy are not the people who review pull requests.

Those two architectures cannot coexist as sources of truth. A service that both
reads a Git-delivered directory and accepts administrative writes has two writers
and guaranteed drift, with the reconciliation loop that implies. One had to win
outright.

Choosing the database gives up what Git provided for free: review, attribution,
history, and rollback. Nothing about a database supplies those by default, and an
enterprise configuration system without them is not deployable — an administrator
must be able to answer "who turned this off, and when".

## Decision

**Postgres is the sole system of record** for target state: manifests, policies,
assignments, and roaming user configuration. The service has **no Git dependency**
— it never clones, polls, or syncs a repository, and GitOps is out of scope as an
architecture.

Every mutation is recorded in an **append-only mutation log** — actor, timestamp,
the submitted change, and the resulting version — that is only ever inserted
into, never updated or deleted. State tables answer *what is configured*; the log
answers *who changed what, when*. This is deliberately not event sourcing: state
is authoritative and is not reconstructed from the log.

Edge environments hold a **replica of their own resolved view** — each device
caches the flattened Policy for its identity's Profile, refreshed by polling with
version/ETag revalidation, bounded by a maximum cache age the Policy itself
carries. Devices never replicate the organisation's whole configuration set.

The directory format survives, demoted, as a **bundle** for import, export,
seeding, and test fixtures. An organisation that prefers review-gated changes can
keep a bundle in a repository and have its own CI call the administrative import
endpoint — that is a consumer of the API, not a mode of the service.

## Consequences

- The mutation log is **not optional and not a follow-up**. It ships with the
  administrative API in the same slice, because it is the replacement for what
  choosing a database gave up.
- Rollback is a product feature to be built (re-apply a previous version from the
  log), not a property inherited from the storage layer.
- The service requires a managed Postgres instance and CI requires a Postgres
  service container. That cost is accepted; it also removes the volume-pinning
  that a file- or SQLite-backed service would impose on a multi-replica
  deployment. SQLite remains only as a test and development store behind the same
  trait.
- Per the standing workspace rule, Postgres access uses `sqlx-core` plus
  `sqlx-postgres` directly. The `sqlx` facade crate is not used: it drags a MySQL
  path and its transitive RSA advisory into the lockfile.
- Convergence is eventual and bounded by cache age, not immediate. There is no
  push channel in the first contract version, so "revoke this setting fleet-wide
  right now" is not a capability the system claims.
