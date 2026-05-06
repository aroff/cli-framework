use serde_json::Value;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliSpecDocument {
    pub schema_version: &'static str,
    pub app: CliSpecApp,
    pub commands: Vec<CliSpecCommand>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CliSpecApp {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliSpecCommand {
    pub path: String,
    pub id: String,
    pub summary: String,
    pub syntax: Option<String>,
    pub category: Option<String>,
    pub hidden: bool,
    pub deprecated: Option<String>,
    pub aliases: Vec<String>,
    pub args: Vec<CliSpecArg>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    pub examples: Vec<String>,
    #[serde(rename = "envVars")]
    pub env_vars: Vec<CliSpecEnvVar>,
    pub exit_codes: Vec<CliSpecExitCode>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CliSpecArg {
    pub name: String,
    pub kind: String,
    pub short: Option<char>,
    pub long: Option<String>,
    pub value_type: String,
    pub cardinality: String,
    pub default: Option<String>,
    pub help: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CliSpecEnvVar {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliSpecExitCode {
    pub code: i32,
    pub description: String,
}
