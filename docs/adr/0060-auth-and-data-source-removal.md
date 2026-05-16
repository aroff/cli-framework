# ADR 0060: Remove `auth` and `data_source` modules

- Date: 2026-05-16
- Status: Accepted
- Issue: 60

## Context

The crate exported two public modules that were not integrated into any dispatch or enforcement
path:

- `cli_framework::auth` (token/RBAC helpers)
- `cli_framework::data_source::DataSource` (async trait)

This created a misleading public surface area: consumers could reasonably assume these were part
of the supported execution model of `cli-framework`, but the runtime does not call them.

## Decision

Remove `auth` and `data_source` from the crate in one breaking cleanup:

- Delete `src/auth/` and remove the `pub mod auth;` export.
- Delete `src/data_source/` and remove the `pub mod data_source;` export and `prelude` re-export.

`RiskEnforcer` remains focused strictly on risk-tier classification and preflight gating; no auth
hook is added.

## Consequences

- Breaking change: downstream crates importing `cli_framework::auth::*` or
  `cli_framework::data_source::DataSource` will fail to compile.
- Migration: consumers should implement auth and data-refresh concerns in their application layer
  (outside `cli-framework`) and wire them into their own command implementations and context.

