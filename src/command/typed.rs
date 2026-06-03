//! Typed handler traits for the register::<T>() API.

use crate::spec::command_tree::CommandSpec;
use crate::spec::value::ArgValue;
use std::collections::HashMap;

/// Implemented by a struct that serves as typed command arguments.
/// The derive macro #[derive(CommandSpec)] generates this automatically.
///
/// Returns a CommandSpec describing the command's args, summary, category, etc.
pub trait IntoCommandSpec {
    fn command_spec() -> CommandSpec;
}

/// Infallible extraction of typed args from a validated ArgValue map.
///
/// The framework guarantees the map has already passed validate_typed_args()
/// before calling this method, so extraction must be infallible.
/// A panic here indicates a framework bug (spec/extraction mismatch).
pub trait FromArgValueMap: Sized {
    fn from_arg_value_map(map: &HashMap<String, ArgValue>) -> Self;
}

/// Convenience alias combining both traits. All derive(CommandSpec) structs
/// automatically implement this.
pub trait TypedArgs: IntoCommandSpec + FromArgValueMap + Send + 'static {}
impl<T: IntoCommandSpec + FromArgValueMap + Send + 'static> TypedArgs for T {}
