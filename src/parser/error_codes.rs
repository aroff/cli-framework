/// Argv contains a command name not present in the registry or command tree.
pub const E_UNKNOWN_COMMAND: &str = "E001";
/// Argv contains a `--flag` not declared in the command's ArgSpec (typed commands only).
pub const E_UNKNOWN_FLAG: &str = "E002";
/// A `Cardinality::Required` arg is absent from the parsed typed args.
pub const E_MISSING_REQUIRED: &str = "E003";
/// A value cannot be coerced to the declared `ArgValueType`.
pub const E_INVALID_VALUE: &str = "E004";
/// A parsed arg's name appears in another present arg's `conflicts_with` list.
pub const E_CONFLICT: &str = "E005";
/// A parsed arg declares `requires = ["x"]` but `"x"` is absent from the parsed args.
pub const E_UNSATISFIED_REQUIRES: &str = "E006";
/// `register_at()` or `register_group()` called with a `CommandPath` already occupied.
pub const E_REGISTRATION_COLLISION: &str = "E007";
/// A `CommandSpec::aliases` entry matches an existing command path or registered alias.
pub const E_ALIAS_CONFLICT: &str = "E008";
/// Returned when `mcp serve` cannot bind the requested address/port.
pub const E_MCP_BIND_FAILED: &str = "E009";
/// Returned when `mcp install` cannot locate the current executable path.
pub const E_MCP_INSTALL_EXE_NOT_FOUND: &str = "E010";
/// Returned when `mcp install` fails to write the agent config entry.
pub const E_MCP_INSTALL_WRITE_FAILED: &str = "E011";
/// Returned when a nested command path is requested but no command is registered at that path.
pub const E_NESTED_COMMAND_NOT_FOUND: &str = "E012";
/// Returned when `completion <shell>` is invoked with an unsupported shell token.
pub const E_UNSUPPORTED_SHELL: &str = "E013";

/// API config: A version name fails validation against `^v\\d+(?:beta\\d+|alpha\\d+)?$`.
pub const E_API_VERSION_INVALID: &str = "E014";
/// API config: Duplicate `ApiVersionName` registered via `ApiServerBuilder::version(v)`.
pub const E_API_DUP_VERSION: &str = "E015";
/// API config: `DefaultVersion::Pinned(v)` references an unregistered version.
pub const E_API_DEFAULT_UNKNOWN: &str = "E016";
/// API config: `ApiServerBuilder::build()` is called with zero registered versions.
pub const E_API_NO_VERSIONS: &str = "E017";
/// API config: A mount path collides with reserved host paths/prefixes.
pub const E_API_MOUNT_COLLISION: &str = "E018";
/// API config: A version name collides with reserved host segments under `/api`.
pub const E_API_VERSION_RESERVED: &str = "E019";
/// API response: `/api/{path}` without a version and `DefaultVersion::None` is configured.
pub const E_API_VERSION_REQUIRED: &str = "E020";
/// API response: Readiness check fails or shutdown is in progress.
pub const E_API_NOT_READY: &str = "E021";
/// Swagger: failed to serialize an app-supplied OpenAPI document at build time.
pub const E_API_SWAGGER_SERIALIZE: &str = "E022";

/// Returned when `doctor --check <id>` specifies an id not in the registered checks.
pub const E_UNKNOWN_DOCTOR_CHECK: &str = "DR003";
/// Returned when `spec --format <format>` specifies an unrecognized format value.
pub const E_UNKNOWN_SPEC_FORMAT: &str = "CS001";

/// Auth: operation not supported by provider (login/logout not implemented).
pub const AUTH001: &str = "AUTH001";
/// Auth: provider-level failure (token acquisition, flow failure, etc.).
pub const AUTH002: &str = "AUTH002";
/// Auth: not authenticated (used by `auth token` when no session exists).
pub const AUTH003: &str = "AUTH003";

/// Config commands: no `ConfigManifest` registered via `AppBuilder::with_config_manifest`.
/// Should not normally occur — the `config` command group is only auto-registered
/// once a manifest has been declared.
pub const CFG001: &str = "CFG001";
/// Config commands: `config profile`/`config refresh` invoked but no `PolicyClient`
/// was registered via `AppBuilder::with_policy_client` — this app has no managed
/// configuration wired in at all.
pub const CFG002: &str = "CFG002";
/// Config commands: `config refresh` could not produce a usable policy outcome —
/// either `PolicyOutcome::Denied` (spec 021 failure mapping: 401-after-retry or
/// 403, which deliberately never falls back to cache) or a `PolicyClientError`
/// (unreachable with no cache, stale cache refused, cache/response corruption).
pub const CFG003: &str = "CFG003";
/// Config commands: `config profile` could not read the cached policy because
/// the cache itself is corrupt or unreadable (`PolicyClientError` from
/// `PolicyClient::cached_policy`) — distinct from `CFG003`, which is a
/// request-time failure (denied access, unreachable server, a bad server
/// response). `CFG004` specifically means "the local, previously-cached
/// policy document on disk could not be parsed," which reads very differently
/// to an operator than "access was denied." `config show` reports the same
/// underlying condition as a non-fatal warning rather than this code, since it
/// can still proceed on local/default values.
pub const CFG004: &str = "CFG004";

/// Telemetry commands: `telemetry set` given a value that is not `off`,
/// `usage`, `diagnostic`, or `debug` (or a required positional arg was
/// somehow absent despite `Cardinality::Required` — defense in depth, since
/// typed-arg validation already rejects both cases before the command body
/// runs). Also used by `telemetry disable`/`enable` for a missing
/// `probe_id`.
pub const TEL001: &str = "TEL001";
/// Telemetry commands: `telemetry disable`/`enable` given a probe id that is
/// not in the built-in registry (`ProbeRegistry::with_builtins`).
pub const TEL002: &str = "TEL002";
/// Telemetry commands: the telemetry settings file could not be written —
/// either the store is `StoreState::Unavailable` (see `TelemetryStore`'s
/// module docs: a directory that could not be created, or an unresolvable
/// platform config directory), or the underlying `ConfigStore` write itself
/// failed for some other `ConfigError` reason.
pub const TEL003: &str = "TEL003";

/// Self-install (ADR 0080): `self install` refused to run as root or under
/// `sudo` without `<APP>_INSTALL_ALLOW_SUDO=1`, or `--system` needs elevation.
pub const SI001: &str = "SI001";
/// Self-install: the target path already holds a binary that no receipt
/// records, and `--force` was not given.
pub const SI002: &str = "SI002";
/// Self-install: the install receipt could not be read, parsed or written.
pub const SI003: &str = "SI003";
/// Self-install: no receipt records this binary (or it records another
/// one), or the binary belongs to a package manager whose own command must
/// be used.
pub const SI004: &str = "SI004";
/// Self-install: `--purge` refused a directory that failed a safety guard.
pub const SI005: &str = "SI005";
/// Self-install: a filesystem or registry operation failed, or no home or
/// data directory could be resolved.
pub const SI006: &str = "SI006";
/// Self-install: an enforced `self_update.*` policy key refused the
/// operation; the message names the key.
pub const SI007: &str = "SI007";
/// Self-install: the release source could not be reached or has no release
/// matching the request.
pub const SI008: &str = "SI008";
/// Self-install: a release failed verification (checksum, signature, unsafe
/// archive entry, or a binary that does not run or reports another version).
pub const SI009: &str = "SI009";
/// Self-install: the request itself was refused: an implicit downgrade,
/// nothing to roll back to, or conflicting arguments.
pub const SI010: &str = "SI010";
/// Self-install: another update holds the bin dir's lock.
pub const SI011: &str = "SI011";
