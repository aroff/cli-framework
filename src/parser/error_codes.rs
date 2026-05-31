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
