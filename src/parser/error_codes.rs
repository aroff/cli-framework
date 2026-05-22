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
