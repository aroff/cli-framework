use crate::command::CommandArgs;
use crate::spec::value::ArgValue;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;

pub fn json_value_to_arg_value(v: &Value) -> Option<ArgValue> {
    match v {
        Value::Bool(b) => Some(ArgValue::Bool(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(ArgValue::Int(i))
            } else {
                n.as_f64().map(ArgValue::Float)
            }
        }
        Value::String(s) => Some(ArgValue::Str(s.clone())),
        Value::Array(arr) => {
            let items: Vec<ArgValue> = arr.iter().filter_map(json_value_to_arg_value).collect();
            Some(ArgValue::List(items))
        }
        _ => None,
    }
}

/// Map JSON tool-call arguments into CLI `CommandArgs` (stringly) and typed args (`ArgValue`).
///
/// Parity contract:
/// - `_positional: [..]` maps to `CommandArgs.positional`
/// - all other keys map to `CommandArgs.named` via stringification
/// - typed values are converted via `json_value_to_arg_value`
pub fn map_tool_args_to_command_args(
    arguments: Value,
) -> Result<(CommandArgs, HashMap<String, ArgValue>)> {
    let obj = match arguments {
        Value::Null => serde_json::Map::new(),
        Value::Object(m) => m,
        other => {
            return Err(anyhow::anyhow!(
                "expected tool arguments to be an object, got {}",
                other
            ));
        }
    };

    let mut named = HashMap::new();
    let mut positional = Vec::new();
    let mut typed = HashMap::new();

    if let Some(Value::Array(pos)) = obj.get("_positional") {
        for v in pos {
            positional.push(match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            });
        }
    }

    for (k, v) in &obj {
        if k == "_positional" {
            continue;
        }
        named.insert(
            k.clone(),
            match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            },
        );
        if let Some(av) = json_value_to_arg_value(v) {
            typed.insert(k.clone(), av);
        }
    }

    Ok((CommandArgs { positional, named }, typed))
}
