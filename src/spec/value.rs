/// Typed argument value payload.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgValue {
    Bool(bool),
    Str(String),
    Int(i64),
    Float(f64),
    /// Validated token from ArgValueType::Enum.
    Enum(String),
    /// For Cardinality::Repeated args.
    List(Vec<ArgValue>),
    /// Occurrence count for Cardinality::Repeated flags.
    Count(u32),
}
