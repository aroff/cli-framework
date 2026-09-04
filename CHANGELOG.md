# Changelog

## [Unreleased]

### Added

- **OS-native keychain `SecretStore` backend**: a new `secrets-keychain` feature (implies
  `secrets`) adding `KeychainSecretStore` under `cli_framework::secrets::keychain`, backed by
  the `keyring` crate (v4, its `v1` compatibility API) — Windows Credential Manager, macOS
  Keychain, or Linux/BSD Secret Service over D-Bus (via keyring's default `v1` feature
  pulling in `zbus-secret-service-keyring-store`, a pure-Rust D-Bus client — no
  `libdbus-dev`/`pkg-config` needed at build time). This is the prerequisite a downstream
  desktop-app PRD depends on for moving plaintext secrets out of its config file and into a
  real OS-backed store.
  - Identity mapping: a caller-supplied `service` prefix (namespaces credentials per
    application, avoiding collisions on a shared machine) paired with the full `/`-joined
    `SecretKey::as_str()` as the account/username — two OS-level fields, documented on
    `KeychainSecretStore` and factored into a standalone `keyring_identity` mapping function
    that's unit-tested without touching a real OS credential store.
  - `get`/`put`/`delete` map onto `keyring::Entry::get_password`/`set_password`/
    `delete_credential`, each off-loaded to `tokio::task::spawn_blocking` (keyring's calls are
    synchronous OS/D-Bus I/O), matching the blocking-call convention `EnvFileSecretStore`
    already uses for its filesystem I/O. `keyring::Error::NoEntry` maps to
    `SecretError::NotFound`; `delete` of an already-absent key is a no-op success, matching
    the trait's documented contract. `rotate` returns `SecretError::NotSupported`, same as
    `InMemorySecretStore` — no backend-generated material to mint here either.
  - Note: `keyring` 4.x's own declared `rust-version` (1.88.0) is newer than this workspace's
    baseline (1.83.0); this only affects consumers who enable `secrets-keychain` specifically.
  - Live-backend trait-conformance coverage
    (`tests/unit/secrets_keychain_conformance.rs`) is opt-in behind `CFW_TEST_KEYCHAIN_LIVE=1`
    and skips (does not fail) otherwise, mirroring the existing `secrets-openbao` precedent —
    this sandbox has a D-Bus session bus but no Secret Service provider registered on it, so
    it can't be exercised here.

- **Config store** (spec 016): a new `config` feature adding writable, versioned
  configuration storage — a byte-level `ConfigBackend` (`FileBackend` with atomic
  temp-file+rename writes and auto-created parent directories, plus a Windows-only
  `RegistryBackend` under `#[cfg(windows)]`) beneath a typed `ConfigStore<T>` that owns
  serialization (JSON by default, TOML selectable), schema-version stamping, migration
  sequencing, and a `reload()`/subscription seam for long-running applications.
  - `AppBuilder::with_config_backend`, `with_config_path`, and `with_config::<T>()` wire a
    store into `build()`, which resolves it once (same point registry freezing happens);
    `AppBuilder::build_with_config::<C, T>()` additionally hands back the resolved typed
    value alongside the built `App`, and `App::config_store::<T>()` recovers the shared
    `Arc<ConfigStore<T>>` for reload/subscribe access.
  - New `AppContext::opt_config_handle(&self) -> Option<&dyn ConfigHandle>` accessor
    (mirrors `opt_registry`'s shape) exposing type-erased `reload()` and raw-JSON
    read/write, since `ConfigStore<T>`'s generic `T` cannot be named on a trait used
    polymorphically.
  - New typed `ConfigError` (`CE001`–`CE009`) covering backend read/write, a read-only
    backend refusing `save`, parse/serialize failures, a failed or missing migration, and
    a stored schema version newer than the running binary (refused, never downgraded).
  - Does not touch the existing `project_config` module (unrelated, CWD-upward-search
    discovery for developer tools) or implement the config manifest / managed-policy layer
    (PRD 021) — this PRD is the local persistence foundation only.

- **Config manifest and the managed-configuration client** (spec 021, ADR 0072/0073):
  builds on the config store above with two independent pieces under `cli_framework::config`.
  - `config::manifest`: a purpose-built, plain-JSON `ConfigManifest` document (fields,
    kinds — boolean/integer/float/string/enumeration/duration/path/url/list/nested
    section — labels, constraints, and the ADR 0073 policy flags `scope`/`platforms`/
    `secret`/`local_only`/`protected`/`manageable`/`enforceable`/`restart_required`). A new
    `#[derive(ConfigManifest)]` alongside `#[derive(CommandSpec)]` in `cli-framework-macros`
    generates it from a config struct's field attributes (`#[config_manifest(app = "...")]`,
    `#[manifest(...)]`); every consumer (the resolver, a provenance query) reads the JSON
    document alone, never the Rust type, so a non-Rust application can author the identical
    document by hand. A one-way `to_json_schema` export is provided for external validators.
  - `config::Policy` / `config::StaleAction`: the plain-data document an organisation serves
    for one profile — `enforced`/`recommended` trees (flat, dotted-leaf-path keyed),
    `policy_version`, `max_cache_age_secs`, `stale_action`. No networking dependency.
  - `config::resolution::resolve`: folds a manifest and six layers (`defaults -> recommended
    -> config file -> environment -> flags -> builder overrides`) plus an `ENFORCED` **veto
    pass** (applied last, over the fully resolved value — not a seventh layer) into resolved
    values and `Provenance` (which layer won, and whether it's locked). Local_only/non-
    manageable/secret fields are dropped (with a structured warning) from a server tree;
    `org`-scoped fields are dropped from `recommended` and `enforceable: false` fields are
    dropped from `enforced`; unknown/type-mismatched server-tree keys are skipped with
    siblings still applying, while the local file keeps this crate's existing
    deny-unknown-fields strictness.
  - New `config-managed` feature (implies `config` + `auth`): `config::managed::PolicyClient`
    fetches `GET /v1/policy/{app}` through the existing `AuthenticatedHttpClient` (reusing its
    401-invalidate-and-retry-once behavior) with ETag revalidation, caches the verbatim policy
    under the platform **data** directory (not config — it's derived state), and applies the
    spec's failure-mapping table: a `401` where retry also fails is treated **identically** to
    `403` and never falls back to cache (this is the safety-critical guard — a token that reads
    as revoked must not be masked by a stale cached policy); `404` runs unmanaged and clears
    the cache; only network failure or `5xx` falls back to cache, subject to the policy's own
    `max_cache_age_secs`/`stale_action`. `config::managed::RoamingConfigClient` reads/writes
    the user-scoped roaming document (`GET`/`PUT /v1/config/{app}`, `If-Match` optimistic
    concurrency), sending only `scope: user` fields regardless of what a caller passes in.
  - Cross-crate: `cli-framework-oidc` gains a `test-support` feature promoting the
    synthesized-OIDC-issuer test helpers (P-256 key generation, JWT minting, a wiremock
    discovery/JWKS/token setup) out of `tests/server_validation.rs` as `#[doc(hidden)] pub`
    functions in `src/test_support.rs`; that test file now consumes the shared versions, and
    `cli-framework`'s own `config-managed` tests depend on it (as a dev-dependency) to mint
    real signed tokens for an end-to-end `OidcClient` (Client Credentials) -> `PolicyClient`
    -> `resolve()` test.
  - **Command surface**: a built-in `config` command group — `config show` (resolved values
    with per-field provenance, `--format table|json`), `config manifest` (the registered
    manifest as pretty-printed JSON, for a release pipeline to publish), `config profile`
    (active org profile + policy version from the last cached fetch, reporting "unmanaged"
    cleanly when nothing was ever fetched), and `config refresh` (forces a policy refetch,
    surfacing `PolicyOutcome`/failure-mapping outcomes verbatim — a `Denied` never falls back
    to cache here either). New `AppBuilder::with_config_manifest` (feature `config`) and
    `AppBuilder::with_policy_client` (feature `config-managed`) register the manifest/client
    the command group renders against, surfaced to handlers via the new
    `AppContext::opt_config_manifest`/`opt_policy_client` accessors; auto-registered in
    `build()` (mirroring the `auth` group's guard) once a manifest is present. New `CFG001`–
    `CFG004` diagnostic codes. Adds `Resolved::entries()` (all resolved leaf paths as one
    `(path, value, provenance)` list) to `config::resolution` and `PolicyClient::cached_policy()`
    (reads the cache with no network call) as the small additive surface these commands needed.
  - **Post-review fixes**: `PolicyClient::fetch` no longer lets a corrupt on-disk cache
    permanently block every future fetch — a cache-read failure now degrades to "as if no
    cache existed" rather than propagating immediately. The failure-mapping `classify()`
    step no longer folds every non-401/403/404 `4xx` (`400`, `409`, `422`, `429`, ...) into
    the same cache-fallback bucket as a genuine `5xx`/network failure; a new
    `HttpFailureClass::ClientError` / `PolicyClientError::ClientError` pair makes those a
    hard, non-cache-eligible error instead. `config show` now reports a visible warning
    (rather than silently degrading to "unmanaged") when the policy cache itself is
    corrupt/unreadable; `config profile` reports the same condition under a new, distinct
    `CFG004` rather than reusing `CFG003` (which is now reserved for request-time failures —
    denied access, an unrecoverable refresh). Most importantly: the manifest/resolver/
    `PolicyClient` machinery above now actually reaches an application's real typed config
    value — `AppBuilder::build_with_config` folds in the policy client's **cached** enforced
    value (network-free) when both a manifest and a policy client are registered, and the new
    `config::managed::refresh_managed_config` (plus `ConfigStore::set_current_and_notify` and
    `config::resolution::unflatten_from_paths`) lets a running application's own explicit
    refresh call — or the built-in `config refresh` command — push a freshly fetched policy
    into the live store, not just print what the server said.
  - Does not implement the config-service (server side, PRDs 022/023) or cryptographic policy
    signing.

- **Config service: read path and storage** (spec 022, ADR 0074/0075): the server side of the
  managed-configuration feature — a new `config-service` feature (implies `config` + `api-server`;
  adds `sqlx-core`/`sqlx-postgres` directly, never the `sqlx` facade crate, which pulls a MySQL
  path and a transitive `rsa` RUSTSEC advisory into the lockfile) under the new
  `cli_framework::config::service` module.
  - `config_service_router(state)` — a **self-authenticating** `axum` router mounted via
    `ApiServerBuilder::mount(...)`: `GET /v1/policy/{app}` (resolved, flattened `Policy` with
    ETag/`If-None-Match` revalidation; `404` = unmanaged), `GET /v1/manifest/{app}`,
    `GET`/`PUT /v1/config/{app}` (roaming document, `If-Match` optimistic concurrency, `412` on
    mismatch, server-side rejection of unknown/machine-scoped/secret fields on write, a size
    cap), and `GET /v1/resolve/{app}` (diagnostic: profile + matching rule, no configuration
    values). Authenticates itself via a per-router `axum::middleware::from_fn_with_state`,
    independent of `ApiServerBuilder::auth()` — that seam applies one auth layer to *every*
    mount in a builder, which would force every other route an embedding app mounts onto the
    same scheme.
  - New crate-local `CallerIdentity` trait (object-safe, one async method, raw `Authorization`
    header in, validated claims as `serde_json::Value` out) — deliberately never names
    `cli-framework-oidc` (that crate already depends on `cli-framework` by path; naming it back
    would be a dependency cycle). `skill/examples/with_config_service` is a runnable adapter
    from `cli-framework-oidc`'s `OidcValidator` to this trait, proving the seam actually
    composes rather than merely type-checking in isolation.
  - `PolicyStore` (manifests, policies, assignment rules) and `UserConfigStore` (roaming
    documents) storage traits, mirroring how `secrets::SecretStore` separates a trait from its
    backends. `FsPolicyStore` reads a read-only bundle directory
    (`manifests/{app}.json`, `policies/{app}/{profile}.toml`, `assignments.toml`) for tests/dev;
    `InMemoryUserConfigStore` is its `UserConfigStore` counterpart.
    `config::service::postgres::{PgPolicyStore, PgUserConfigStore}` are the real backend, with a
    small hand-rolled SQL migration runner (embedded `.sql`, a `schema_migrations` table,
    "refuse rather than downgrade" if the database is ahead of the binary) serialized against
    concurrent replicas via a Postgres advisory lock.
  - Assignment-rule evaluation (`{claim_path, operator, value, profile}`, operators
    `equals`/`contains`/`exists`, first match wins, optional default profile) and single-parent
    inheritance (deep-merged server-side, cycle-rejected at startup and defensively at read
    time) so the wire `Policy` never represents inheritance at all. Startup validation reuses
    — rather than reimplements — spec 021's own resolver drop-reason rules
    (`server_tree_drop_reason_recommended`/`_enforced`, now `pub(crate)` for this one call
    site) for unknown-field/type-mismatch/secret/local_only/org-scope/enforceable-false
    conformance checking.
  - CI gets its first-ever service container: a `postgres:16` service in `.github/workflows/ci.yml`
    backs a new `Test (config-service)` step; the Postgres half of the trait-conformance suite
    (`tests/integration/config_service_postgres_conformance.rs`) triggers purely on `DATABASE_URL`
    being set (present in CI, usually absent locally) rather than a separate opt-in flag, unlike
    the pre-existing `testcontainers`-based OpenBao precedent, which never runs in CI at all.
  - Out of scope: administrative writes, the mutation log, and import/export endpoints
    (PRD 023); this slice's storage is read-only at the API surface.

- **Config service: administrative write API and mutation log** (spec 023, ADR 0075): an
  administrative HTTP surface under `/v1/admin/*`, mounted on the same
  `config_service_router`, for publishing manifests, replacing/patching policy documents,
  reading change history and restoring a prior version, replacing assignment rules, and
  whole-store bundle export/import.
  - Every write is authorized by a two-gate model: a valid `CallerIdentity` (`401`), then a
    configurable admin-role `AssignmentRule` (`403`) reusing the existing assignment-rule
    evaluator — default rule requires `realm_access.roles` to contain `"config-admin"`,
    overridable via `ConfigServiceState::with_admin_rule`.
  - New `PolicyAdminStore` trait (`put_manifest`, `put_policy`, `assignment_rules_version`,
    `put_assignment_rules`, `policy_history`, `import_bundle`), implemented only by
    `config::service::postgres::PgPolicyStore`; wired in via the new
    `ConfigServiceState::with_admin_store`. Without it, every `/v1/admin/*` route responds
    `500` (a deployment-configuration gap), not `404`.
  - `PATCH /v1/admin/policy/{app}/{profile}` applies hand-rolled RFC 7386 JSON Merge Patch
    independently to the `enforced` and `recommended` trees, so one request can move a field
    between them. Every write except `import_bundle` is optimistic-concurrency-checked via
    `If-Match`/`expected_version`, mapped to `412` on mismatch — the same mechanism the
    device-facing roaming-config write already uses.
  - `POST .../history/{version}/restore` writes a **new** version whose content matches a
    prior one (a forward change, never an in-place history rewrite). New migration
    `002_admin_mutation_log.sql` adds an append-only `mutation_log` table with **no foreign
    key** to `manifest`/`policy`/`assignment` — by design, so a row survives deletion of the
    resource it describes — plus `assignment_set`, a per-app version counter the `assignment`
    table itself has no column for, needed to give `/v1/admin/assignments/{app}` an
    `If-Match`/`ETag` basis. Each `mutation_log` row records `kind`
    (`manifest_put`/`policy_put`/`policy_patch`/`policy_restore`/`assignments_put`/`import`),
    `actor`, `occurred_at`, exactly what the caller `submitted` (never the merged/restored
    result), and a `resulting_document` snapshot.
  - `GET /v1/admin/export` / `POST /v1/admin/import` speak the existing bundle-directory
    format; `FsPolicyStore::load` is the only parser (no second, hand-rolled one).
    `import_bundle` validates the entire bundle against its own contents before writing
    anything, then commits every table plus one `mutation_log` row per app in a single
    transaction.
  - Validation reuses `validate_stored_policy` + `inherit::resolve_chain` unchanged; two
    checks inlined in `validate_all` were extracted into `pub(crate)` helpers so the admin
    write path and startup validation share one implementation rather than risking drift.
  - **Post-review fixes (#133)**: a manifest write no longer strands already-stored policies
    against a validation gap that could refuse the service on next restart, and policy writes
    no longer validate against a plain unlocked read taken before their own transaction (which
    let a concurrent manifest change race past validation) — both closed by locking the
    manifest and policy rows for an app in a consistent order and re-validating against the
    locked state before committing; `import_bundle` reuses the same discipline. Also hardens
    bundle export/import scratch directories against a stale-directory reuse on PID collision
    (fails loudly instead of silently mixing in leftover files), and fixes an
    assignment-rule-wiping bug in partial bundle imports.
  - **Post-review fixes (#134)**, from a final whole-system adversarial review across the
    merged spec 016/021/022/023 feature (no criticals; three real gaps): declared field
    `constraints` (`min`/`max`/`allowed_values`) — previously carried in the manifest and
    rendered into a JSON Schema document for UIs, but never enforced anywhere server-side —
    are now enforced in `validate_stored_policy` (reused by every write path) and by the
    roaming user-config write handler; the resolver's own advisory-only treatment of
    constraints is unchanged. Fixed a lock-order inversion between `import_bundle` and
    `put_assignment_rules` (opposite acquisition order for the `assignment_set` and
    `assignment` locks — a concurrency test confirms this causes a real, deterministic
    duplicate-key failure when the two race, not just a theoretical deadlock) by locking
    `assignment_set` first in both, matching the existing manifest-then-policy discipline
    elsewhere. Roaming user-config writes now reject `local_only` fields too (previously only
    `secret` and non-`user`-scoped fields were rejected), closing a path for a
    device-bootstrap-only field to roam to a user's other devices via the server.

- **`ArgSpec::min_occurs`** (`Option<usize>`, defaults to `None`) declares the minimum arity of a
  `Cardinality::Repeated` argument. `Repeated` alone cannot distinguish `--header` (zero or more)
  from a `<skill-ids>...` positional (one or more); `min_occurs: Some(1)` or higher now lists the
  argument in `inputSchema.required` — previously only `Cardinality::Required` reached that array,
  so a mandatory repeated argument was indistinguishable from an optional one — and adds
  `minItems` (arrays) / `minimum` (occurrence-count flags). `None` and `Some(0)` both keep the
  pre-existing zero-or-more behavior, and the field is ignored for `Required` and `Optional`.
  Schema-level only: CLI parsing arity is unchanged. Also adds `ArgSpec::is_schema_required()`,
  the single predicate `CommandSpec::to_json_schema` now uses to build `required`.
  Additive for consumers that construct `ArgSpec` with `..Default::default()` (every site in this
  repo and in `#[derive(CommandSpec)]` does); a consumer using an exhaustive struct literal with
  no `..Default::default()` must add the field.

### Fixed

- **MCP tool schemas dropped argument descriptions and never marked one-or-more arguments as
  required**, which is the half of the MCP surface an agent can actually read.
  `ArgSpec::to_json_schema_property` returned early for `Cardinality::Repeated`, bypassing the
  shared `description` insertion at the end of the function, so every repeated argument reached
  `tools/list` as a bare `{"type":"array","items":{"type":"string"}}` (or `{"type":"integer"}`
  for a repeated flag) with `ArgSpec.help` discarded. Repeated shapes now build through the same
  path as scalars, so `help` is emitted as the property `description` for every argument shape.
  Observed downstream: `fastskill_remove` exposed a `skill-ids` property with no description and
  no `required` array, so the only way to discover the key (including its casing) was to read the
  raw schema and guess.

- **The generated bash completion script was structurally broken and worse than no completion at
  all.** `emit_completion_script`'s bash branch read `COMP_WORDS[1]` — hardcoded to the *first*
  argument — where bash requires `COMP_WORDS[COMP_CWORD]`, the index of the word the cursor is
  actually on. Everything past the first word was therefore matched against word 1, and the only
  candidate list ever offered was the flat set of top-level verbs: `app repos <TAB>` completed to
  `repos`, and no subcommand or flag was ever completed. Because the compspec always produced a
  match, it also suppressed the shell's default filename completion. The bash branch now emits
  `COMP_WORDS[COMP_CWORD]`, rebuilds the command path from the non-flag words before the cursor,
  and selects per-level candidates from a registry-derived model (`build_completion_model`) that
  carries every group's subcommands and every leaf command's own flags (long, short, and
  `--help`), with hidden commands and hidden groups — and anything nested under a hidden group —
  omitted at every level. The compspec is registered `complete -o default`, so filename
  completion still applies where the framework has no candidates. `zsh`, `fish` and `powershell`
  output is unchanged. Internal-only signature change: the `pub(crate)` helpers
  `emit_completion_script`/`visible_top_level_commands` now take a `CompletionModel`; the public
  `App::emit_completion` is untouched.

- **`mcp install` and `mcp register` were one command registered twice.** `build()` cloned the
  install command, rewrote its id to `register`, and registered it a second time at
  `mcp/register`, so `mcp --help` offered two primary verbs with byte-identical descriptions
  ("Install this app as an MCP server in an agent configuration") and nothing to choose
  between them. `register` is now a **hidden alias** declared on the install command's spec
  (`CommandSpec.hidden_aliases`) rather than a second registration: `mcp register …` keeps
  working unchanged — clap resolves the alias to `install` — but it is no longer listed in
  `mcp --help` beside `install`, and the command tree holds one install command instead of
  two. The alias is **deprecated**; prefer `mcp install`. It is kept for one release so
  downstream binaries and scripts that already call `mcp register` do not break, and may be
  removed after that.
  - Registry-visible consequence: `command_registry().resolve(["mcp", "register"])` now
    returns `None` (the alias is carried on `mcp/install`'s spec), and the `spec` command's
    document lists a single install entry instead of two identical ones.

## [0.5.4] — 2026-06-13

### Added

- MCP tool results can now carry **`structuredContent`** distinct from the model-facing
  `content` text (CF-7). A command's `execute` attaches it via the new
  `AppContext::framework_set_structured_content(serde_json::Value)`; the MCP dispatch maps it to
  `CallToolResult.structured_content` while the text emitted via `framework_println` remains the
  `content`. This lets a tool return a large payload (e.g. server-rendered HTML for an MCP-Apps
  iframe) to the host without dumping it into the model's text context.
  - New `tool_bridge::BridgeOutput { text, structured }` and
    `CommandAsToolBridge::invoke_structured(...)`; the existing `invoke(...)` is unchanged
    (returns text only) for chat callers.
  - The MCP dispatch context now **captures** a command's `framework_println` output (previously it
    went to the server's stdout and the tool reported only `"OK"`).

## [0.5.3] — 2026-06-13

### Added

- `ResourceRegistry` now supports **prefix providers** (`register_prefix`): a single provider
  serves every URI under a base (e.g. `ui://es/invoice/detail/` serving `ui://es/invoice/detail/<id>`),
  receiving the full requested URI so it can render per-record resources on demand. `read` resolves
  an exact registration first, else the longest matching prefix (CF-6b). Additive; existing exact
  registration is unchanged.

## [0.5.1] — 2026-06-13

### Added

- MCP serve path now threads a populated `ResourceRegistry` end-to-end, so registered `ui://…`
  resources are actually served (CF-6). Previously `ResourceRegistry` and
  `CliFrameworkHandler::with_resource_registry` existed but no public serve entry point ever called
  `with_resource_registry`, so a populated registry had no route to the served handler.
  - New consumer-facing slot: `AppBuilder::with_mcp_resource_registry(Arc<ResourceRegistry>)`. The
    auto-registered `mcp serve` command now serves those resources over **both** stdio and HTTP
    transports (`resources/list` + `resources/read`).
  - New HTTP-side seam for apps that mount MCP into their own Axum router:
    `mcp::build_mcp_axum_router_with_resources(...)` (the existing `build_mcp_axum_router` delegates
    to it with an empty registry).
  - New lower-level serve variants that accept an `Arc<ResourceRegistry>`:
    `serve_mcp_stdio_opts_with_resources`, `serve_mcp_with_gate_opts_with_resources`,
    `transport_stdio::start_stdio_with_resources`,
    `transport_http::start_streamable_http_with_resources`,
    `transport_http::mcp_axum_router_with_resources`.
  - All changes are additive and backward compatible: existing serve signatures are unchanged and
    default to an empty registry (a tools-only server, exactly as before).

## [0.5.0] — 2026-06-12

### Added

- MCP generic per-tool `_meta` passthrough: `Command::with_meta(serde_json::Value)` attaches an
  opaque value emitted verbatim as the tool's top-level `_meta` on `tools/list`. cli-framework does
  not inspect it — the consumer owns the entire shape (e.g. UI metadata, but the framework stays
  concept-free). `Command::with_visibility(Vec<String>)` continues to tag app-only tools (the one
  field cli-framework acts on; rides in `_meta.visibility`).
- MCP generic resource serving stays in-scope but concept-free: `UiResource::with_meta(
  serde_json::Value)` attaches an opaque per-resource `_meta` emitted verbatim at
  `contents[]._meta` in `resources/read`. The `ResourceRegistry` and `CliFrameworkHandler`
  resource seams are unchanged.
- Built-in `completion <shell>` command (bash/zsh/fish/powershell) auto-registered by `AppBuilder::build()`. Apps that already define `completion` can opt out via `AppBuilder::without_completion()`.
- `api-server` feature: versioned Axum API hosting under `/api/{version}/...` with fixed `/healthz` + `/readyz` endpoints and graceful shutdown coordination.
- `api-swagger` feature: runtime OpenAPI spec endpoint and embedded Swagger UI — serves each version's app-supplied document at `GET /api/{version}/openapi.json` (with `servers:` patch) and renders a version-switchable Swagger UI at `GET /api/docs` with no CDN dependency.
- `ApiServerBuilder::root_fallback(axum::Router)`: attach a catch-all router to handle requests not matched by any framework or application route. Intended for serving a SPA or static assets at the root on the same listener as the versioned API. Receives the configured `CorsLayer` (if any); auth is intentionally not applied by default. Framework routes always take priority over the fallback.
- `ApiServerBuilder::health_version(impl Into<String>)`: override the version string reported by `GET /healthz`. By default `/healthz` reports the framework's own crate version (`env!("CARGO_PKG_VERSION")`), fixed at cli-framework's compile time; consumers can call this to make `/healthz` report THEIR version instead. Back-compatible: when unset, `/healthz` reports the framework version exactly as before.

### Fixed

- `AsyncRetryExecutor` now honors `RetryPolicy::retry_on_timeout`. Previously the flag was ignored
  and a per-attempt timeout was always retried; with `retry_on_timeout(false)` a timed-out attempt
  now fails immediately without further retries. Non-timeout operation errors continue to retry
  regardless of the flag. Added a `unit_retry` test suite covering policy backoff math, the
  sync/async executors, the async error classifier, and timeout handling.

### Breaking

- Removed the typed MCP-Apps UI vocabulary from the core command model. cli-framework is a generic
  MCP transport and must not know UI concepts. Removed `command::UiToolMeta`, `command::UiCsp`, the
  `Command::ui` field, `Command::with_ui`, and their prelude re-exports. Replaced by the opaque
  `Command::meta: Option<serde_json::Value>` + `Command::with_meta` passthrough. Likewise
  `UiResource::csp: Option<UiCsp>` / `UiResource::with_csp` are replaced by
  `UiResource::meta: Option<serde_json::Value>` / `UiResource::with_meta`. Consumers that previously
  built `with_ui(UiToolMeta { resource_uri, csp, prefer_app })` now pass the entire `_meta` value
  themselves, e.g. `with_meta(json!({"ui":{"resourceUri":"…","csp":{…},"preferApp":true}}))`.
- Removed `cli_framework::auth` and `cli_framework::data_source::DataSource` (and the prelude
  re-export). These modules were not integrated into command dispatch; consumers should implement
  auth and data-refresh concerns in their application layer.

## [0.4.0] — 2026-05-04

### Breaking

- `clap-dispatch` is now included in the `default` feature set. Consumers using
  `default-features = false` who relied on the legacy hand-rolled argv loop must add
  `features = ["clap-dispatch"]` to retain access to the Clap path, or adopt `CommandSpec` on
  their commands.

### Deprecated

- The `clap-dispatch` feature flag is now a no-op (Clap dispatch is always active). The flag is
  retained for one release cycle to avoid breaking consumers who list it explicitly. It will be
  removed in v0.5.0.

### Removed

- The hand-rolled `run_with_args` implementation (formerly behind
  `#[cfg(not(feature = "clap-dispatch"))]` in `src/app/builder.rs`). Only the Clap-backed path
  remains.

### Migration

Consumers using `default-features = false` who relied on the legacy argv loop must either:

1. Add `features = ["clap-dispatch"]` to their `cli-framework` dependency, **or**
2. Add `CommandSpec` to their commands to get full Clap integration.

Unknown flags now produce a structured `Diagnostic` with code `E_UNKNOWN_FLAG` on stderr instead
of being silently ignored.
