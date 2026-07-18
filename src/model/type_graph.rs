use serde::{Deserialize, Serialize};

pub type TypeRef = u32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Actor {
    Service { service: TypeRef },
    Class { class: TypeRef },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TypeNode {
    Primitive {
        primitive: PrimitiveType,
    },
    Opt {
        inner: TypeRef,
    },
    Vec {
        inner: TypeRef,
    },
    Record {
        fields: Vec<Field>,
    },
    Variant {
        fields: Vec<Field>,
    },
    Func {
        args: Vec<TypeRef>,
        results: Vec<TypeRef>,
        mode: MethodMode,
    },
    Service {
        methods: Vec<ServiceMethod>,
    },
    Class {
        init: Vec<TypeRef>,
        service: TypeRef,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveType {
    Null,
    Bool,
    Nat,
    Int,
    Nat8,
    Nat16,
    Nat32,
    Nat64,
    Int8,
    Int16,
    Int32,
    Int64,
    Float32,
    Float64,
    Text,
    Reserved,
    Empty,
    Principal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MethodMode {
    /// The absence of a Candid annotation, made explicit in the Contract.
    Update,
    Query,
    CompositeQuery,
    Oneway,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    /// The authoritative Candid label ID: numeric label or `idl_hash(name)`.
    pub id: u32,
    #[serde(rename = "type")]
    pub ty: TypeRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceMethod {
    /// Method text is required to invoke a service. `id` is retained as the
    /// authoritative Candid hash for reflection and validation.
    pub name: String,
    pub id: u32,
    #[serde(rename = "function")]
    pub function: TypeRef,
}
