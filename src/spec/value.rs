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

impl std::fmt::Display for ArgValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArgValue::Bool(b) => write!(f, "{}", b),
            ArgValue::Str(s) => write!(f, "{}", s),
            ArgValue::Int(i) => write!(f, "{}", i),
            ArgValue::Float(fl) => write!(f, "{}", fl),
            ArgValue::Enum(e) => write!(f, "{}", e),
            ArgValue::Count(c) => write!(f, "{}", c),
            ArgValue::List(_) => {
                unimplemented!("use args.get_list(); List cannot be round-tripped through Display")
            }
        }
    }
}
