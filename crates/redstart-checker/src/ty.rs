//! Redstart's resolved type system.
//!
//! `RTy` is the resolved counterpart of a syntactic `TypeExpr`. It is shared by
//! the checker (to validate handler bodies) and by codegen (to choose correct
//! AssemblyScript lowerings) — one source of truth for "what type is this".

use redstart_parser::ast::TypeExpr;
use std::collections::HashMap;

/// A resolved Redstart type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RTy {
    BigInt,
    BigDecimal,
    Bytes,
    Address,
    String,
    Boolean,
    Int,
    /// An entity reference (stored as its id in graph-ts).
    Entity(String),
    /// `Option<T>` — nullable.
    Option(Box<RTy>),
    /// A list `[T]`.
    List(Box<RTy>),
    /// A bound contract instance for ABI `name`.
    Contract(String),
    /// `Result<T, CallRevert>` — the type of every contract call.
    Result(Box<RTy>),
    /// The handler's event object (`event`).
    Event,
    /// `event.params`.
    Params,
    /// A call handler's call object (`call`).
    Call,
    /// `call.inputs`.
    CallInputs,
    /// `call.outputs`.
    CallOutputs,
    /// `event.block` / `block` (block handler) / `call.block`.
    Block,
    /// `event.transaction` / `call.transaction`.
    Transaction,
    /// Anything we couldn't resolve.
    Unknown,
}

impl RTy {
    /// Whether this is `BigInt`.
    #[must_use]
    pub fn is_bigint(&self) -> bool {
        matches!(self, RTy::BigInt)
    }
    /// Whether this is `BigDecimal`.
    #[must_use]
    pub fn is_bigdecimal(&self) -> bool {
        matches!(self, RTy::BigDecimal)
    }
    /// Whether this is an `Option<_>`.
    #[must_use]
    pub fn is_option(&self) -> bool {
        matches!(self, RTy::Option(_))
    }
}

/// Field types for one entity.
#[derive(Debug, Clone, Default)]
pub struct EntityInfo {
    /// Field name -> resolved type.
    pub fields: HashMap<String, RTy>,
}

/// Map a Solidity ABI type to a resolved type.
///
/// Integer *width* matters: `graph codegen` lowers the narrow widths to a native
/// `i32` and everything wider to a `BigInt`, so typing a `uint8` as `BigInt`
/// emits `.equals(1)` on an `i32` — code that doesn't compile. The width table
/// here is graph-cli's, exactly: `int8…int32` and `uint8…uint24` are `i32`.
#[must_use]
pub fn sol_to_rty(sol: &str) -> RTy {
    // `uint8[]`, `address[3]` — an array of the element type.
    if let Some(open) = sol.rfind('[') {
        if sol.ends_with(']') {
            return RTy::List(Box::new(sol_to_rty(&sol[..open])));
        }
    }
    if sol == "address" {
        RTy::Address
    } else if sol == "bool" {
        RTy::Boolean
    } else if sol == "string" {
        RTy::String
    } else if let Some(bits) = sol.strip_prefix("uint") {
        if matches!(bits, "8" | "16" | "24") {
            RTy::Int
        } else {
            RTy::BigInt
        }
    } else if let Some(bits) = sol.strip_prefix("int") {
        if matches!(bits, "8" | "16" | "24" | "32") {
            RTy::Int
        } else {
            RTy::BigInt
        }
    } else if sol.starts_with("bytes") {
        RTy::Bytes
    } else {
        RTy::Unknown
    }
}

/// The set of built-in scalar type names.
#[must_use]
pub fn is_scalar(name: &str) -> bool {
    matches!(
        name,
        "BigInt"
            | "BigDecimal"
            | "Bytes"
            | "Address"
            | "String"
            // Both spellings: `Bool` is Redstart's, `Boolean` is what every
            // schema.graphql being ported from says.
            | "Bool"
            | "Boolean"
            | "Int"
            | "Int8"
            | "Timestamp"
            | "ID"
    )
}

/// Resolve a syntactic type to a resolved type, given the known entity names.
#[must_use]
pub fn resolve_type(ty: &TypeExpr, entities: &HashMap<String, EntityInfo>) -> RTy {
    match ty {
        TypeExpr::List { elem, .. } => RTy::List(Box::new(resolve_type(elem, entities))),
        TypeExpr::Generic { base, args, .. } => {
            let name = base.simple_name().unwrap_or("");
            match name {
                "Option" => RTy::Option(Box::new(
                    args.first()
                        .map_or(RTy::Unknown, |t| resolve_type(t, entities)),
                )),
                "Id" => args
                    .first()
                    .map_or(RTy::Bytes, |t| resolve_type(t, entities)),
                "List" => RTy::List(Box::new(
                    args.first()
                        .map_or(RTy::Unknown, |t| resolve_type(t, entities)),
                )),
                _ => RTy::Unknown,
            }
        }
        TypeExpr::Path { .. } => match ty.simple_name().unwrap_or("") {
            "BigInt" => RTy::BigInt,
            "BigDecimal" => RTy::BigDecimal,
            "Bytes" => RTy::Bytes,
            "Address" => RTy::Address,
            "String" => RTy::String,
            "Bool" | "Boolean" => RTy::Boolean,
            "Int" => RTy::Int,
            other if entities.contains_key(other) => RTy::Entity(other.to_string()),
            _ => RTy::Unknown,
        },
    }
}
