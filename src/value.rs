use crate::limits::Limits;
use crate::model::{Contract, PrimitiveType, TypeNode, TypeRef};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTypeRef {
    pub contract_id: String,
    #[serde(rename = "type")]
    pub type_ref: TypeRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractMethodRef {
    pub contract_id: String,
    pub method: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostFieldValue {
    pub id: u32,
    pub value: HostValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostValue {
    Null,
    Bool { value: bool },
    Nat { value: String },
    Int { value: String },
    Nat8 { value: u8 },
    Nat16 { value: u16 },
    Nat32 { value: u32 },
    Nat64 { value: String },
    Int8 { value: i8 },
    Int16 { value: i16 },
    Int32 { value: i32 },
    Int64 { value: String },
    Float32 { bits: String },
    Float64 { bits: String },
    Text { value: String },
    Reserved,
    Principal { value: String },
    Opt { value: Option<Box<HostValue>> },
    Vec { values: Vec<HostValue> },
    Record { fields: Vec<HostFieldValue> },
    Variant { id: u32, value: Box<HostValue> },
    Service { principal: String },
    Func { principal: String, method: String },
}

impl HostValue {
    pub fn from_json_with_limits(input: &str, limits: &Limits) -> Result<Self, HostValueJsonError> {
        if input.len() > limits.max_value_bytes {
            return Err(HostValueJsonError::Limit {
                limit: limits.max_value_bytes,
                observed: input.len(),
            });
        }
        serde_json::from_str(input)
            .map_err(|error| HostValueJsonError::Malformed(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostValueJsonError {
    Malformed(String),
    Limit { limit: usize, observed: usize },
}

impl fmt::Display for HostValueJsonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(message) => write!(formatter, "malformed HostValue JSON: {message}"),
            Self::Limit { limit, observed } => write!(
                formatter,
                "HostValue JSON uses {observed} bytes; limit is {limit}"
            ),
        }
    }
}

impl std::error::Error for HostValueJsonError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostValueViolation {
    pub code: String,
    pub path: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_limit: Option<crate::ResourceLimitInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostValueValidationError {
    pub violations: Vec<HostValueViolation>,
}

impl fmt::Display for HostValueValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "HostValue validation failed with {} violation(s)",
            self.violations.len()
        )
    }
}

impl std::error::Error for HostValueValidationError {}

pub fn validate_host_value(
    contract: &Contract,
    selector: &ContractTypeRef,
    value: &HostValue,
    limits: &Limits,
) -> Result<(), HostValueValidationError> {
    if selector.contract_id != contract.contract_id() {
        return Err(single(
            "value_contract_id_mismatch",
            "$",
            format!(
                "expected Contract {}, found {}",
                contract.contract_id(),
                selector.contract_id
            ),
        ));
    }
    if selector.type_ref as usize >= contract.types().len() {
        return Err(single(
            "value_type_ref_out_of_bounds",
            "$",
            format!(
                "type reference {} is outside the Contract",
                selector.type_ref
            ),
        ));
    }

    let mut work = vec![(selector.type_ref, value, "$".to_string(), 0usize)];
    let mut elements = 0usize;
    let mut bytes = 0usize;
    while let Some((reference, value, path, depth)) = work.pop() {
        if limits.deadline_exceeded() {
            return Err(single(
                "operation_deadline_exceeded",
                path,
                "HostValue validation deadline has elapsed",
            ));
        }
        if depth > limits.max_value_depth {
            return Err(resource_single(
                "value_depth",
                limits.max_value_depth,
                depth,
                path,
                format!("value depth exceeds limit {}", limits.max_value_depth),
            ));
        }
        elements = elements.saturating_add(1);
        if elements > limits.max_value_elements {
            return Err(resource_single(
                "value_elements",
                limits.max_value_elements,
                elements,
                path,
                format!("value elements exceed limit {}", limits.max_value_elements),
            ));
        }
        bytes = bytes.saturating_add(value_string_bytes(value));
        match (&contract.types()[reference as usize], value) {
            (TypeNode::Primitive { primitive }, value) => {
                validate_primitive(*primitive, value, &path)?;
            }
            (TypeNode::Opt { inner }, HostValue::Opt { value }) => {
                if let Some(value) = value {
                    work.push((*inner, value, format!("{path}.value"), depth + 1));
                }
            }
            (TypeNode::Vec { inner }, HostValue::Vec { values }) => {
                for (index, value) in values.iter().enumerate().rev() {
                    work.push((*inner, value, format!("{path}.values[{index}]"), depth + 1));
                }
            }
            (TypeNode::Record { fields }, HostValue::Record { fields: values }) => {
                let mut by_id = BTreeMap::new();
                for field in values {
                    if by_id.insert(field.id, &field.value).is_some() {
                        return Err(single(
                            "duplicate_host_field",
                            &path,
                            format!("record field ID {} occurs more than once", field.id),
                        ));
                    }
                }
                let expected: BTreeSet<_> = fields.iter().map(|field| field.id).collect();
                let actual: BTreeSet<_> = by_id.keys().copied().collect();
                if expected != actual {
                    return Err(single(
                        "record_field_set_mismatch",
                        &path,
                        format!("expected field IDs {expected:?}, found {actual:?}"),
                    ));
                }
                for field in fields.iter().rev() {
                    work.push((
                        field.ty,
                        by_id[&field.id],
                        format!("{path}.fields[{}]", field.id),
                        depth + 1,
                    ));
                }
            }
            (TypeNode::Variant { fields }, HostValue::Variant { id, value }) => {
                let Some(field) = fields.iter().find(|field| field.id == *id) else {
                    return Err(single(
                        "unknown_variant_id",
                        &path,
                        format!("variant ID {id} does not exist in the expected type"),
                    ));
                };
                work.push((field.ty, value, format!("{path}.value"), depth + 1));
            }
            (TypeNode::Service { .. }, HostValue::Service { principal }) => {
                validate_principal(principal, &path)?;
            }
            (TypeNode::Func { .. }, HostValue::Func { principal, method }) => {
                validate_principal(principal, &path)?;
                if method.is_empty() {
                    return Err(single(
                        "empty_function_method",
                        &path,
                        "function method names must not be empty",
                    ));
                }
            }
            (TypeNode::Class { .. }, _) => {
                return Err(single(
                    "class_has_no_host_value",
                    &path,
                    "service constructors are not first-class Candid values",
                ));
            }
            (expected, actual) => {
                return Err(single(
                    "host_value_kind_mismatch",
                    &path,
                    format!(
                        "expected {}, found {}",
                        type_node_kind(expected),
                        host_value_kind(actual)
                    ),
                ));
            }
        }
        if bytes > limits.max_value_bytes {
            return Err(resource_single(
                "value_bytes",
                limits.max_value_bytes,
                bytes,
                &path,
                format!("value bytes exceed limit {}", limits.max_value_bytes),
            ));
        }
    }
    Ok(())
}

fn validate_primitive(
    primitive: PrimitiveType,
    value: &HostValue,
    path: &str,
) -> Result<(), HostValueValidationError> {
    let valid = match (primitive, value) {
        (PrimitiveType::Null, HostValue::Null)
        | (PrimitiveType::Bool, HostValue::Bool { .. })
        | (PrimitiveType::Nat8, HostValue::Nat8 { .. })
        | (PrimitiveType::Nat16, HostValue::Nat16 { .. })
        | (PrimitiveType::Nat32, HostValue::Nat32 { .. })
        | (PrimitiveType::Int8, HostValue::Int8 { .. })
        | (PrimitiveType::Int16, HostValue::Int16 { .. })
        | (PrimitiveType::Int32, HostValue::Int32 { .. })
        | (PrimitiveType::Reserved, HostValue::Reserved) => true,
        (PrimitiveType::Nat, HostValue::Nat { value }) => canonical_nat(value),
        (PrimitiveType::Int, HostValue::Int { value }) => canonical_int(value),
        (PrimitiveType::Nat64, HostValue::Nat64 { value }) => {
            canonical_nat(value) && value.parse::<u64>().is_ok()
        }
        (PrimitiveType::Int64, HostValue::Int64 { value }) => {
            canonical_int(value) && value.parse::<i64>().is_ok()
        }
        (PrimitiveType::Float32, HostValue::Float32 { bits }) => canonical_hex(bits, 8),
        (PrimitiveType::Float64, HostValue::Float64 { bits }) => canonical_hex(bits, 16),
        (PrimitiveType::Text, HostValue::Text { .. }) => true,
        (PrimitiveType::Principal, HostValue::Principal { value }) => {
            validate_principal(value, path)?;
            true
        }
        (PrimitiveType::Empty, _) => {
            return Err(single(
                "empty_has_no_value",
                path,
                "the Candid empty type has no constructible HostValue",
            ));
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(single(
            "host_value_kind_mismatch",
            path,
            format!(
                "expected primitive {primitive:?}, found {} or a non-canonical representation",
                host_value_kind(value)
            ),
        ))
    }
}

fn value_string_bytes(value: &HostValue) -> usize {
    match value {
        HostValue::Nat { value }
        | HostValue::Int { value }
        | HostValue::Nat64 { value }
        | HostValue::Int64 { value }
        | HostValue::Text { value }
        | HostValue::Principal { value } => value.len(),
        HostValue::Float32 { bits } | HostValue::Float64 { bits } => bits.len(),
        HostValue::Service { principal } => principal.len(),
        HostValue::Func { principal, method } => principal.len().saturating_add(method.len()),
        _ => 0,
    }
}

fn canonical_nat(value: &str) -> bool {
    value == "0"
        || (!value.starts_with('0')
            && !value.is_empty()
            && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn canonical_int(value: &str) -> bool {
    if let Some(magnitude) = value.strip_prefix('-') {
        magnitude != "0" && canonical_nat(magnitude)
    } else {
        canonical_nat(value)
    }
}

fn canonical_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_principal(value: &str, path: &str) -> Result<(), HostValueValidationError> {
    candid_parser::Principal::from_text(value).map_err(|error| {
        single(
            "invalid_principal",
            path,
            format!("invalid principal {value:?}: {error}"),
        )
    })?;
    Ok(())
}

fn single(
    code: impl Into<String>,
    path: impl Into<String>,
    message: impl Into<String>,
) -> HostValueValidationError {
    HostValueValidationError {
        violations: vec![HostValueViolation {
            code: code.into(),
            path: path.into(),
            message: message.into(),
            resource_limit: None,
        }],
    }
}

fn resource_single(
    resource: &str,
    limit: usize,
    observed: usize,
    path: impl Into<String>,
    message: impl Into<String>,
) -> HostValueValidationError {
    HostValueValidationError {
        violations: vec![HostValueViolation {
            code: "resource_limit_exceeded".to_string(),
            path: path.into(),
            message: message.into(),
            resource_limit: Some(crate::ResourceLimitInfo {
                resource: resource.to_string(),
                limit,
                observed,
            }),
        }],
    }
}

fn type_node_kind(node: &TypeNode) -> &'static str {
    match node {
        TypeNode::Primitive { .. } => "primitive",
        TypeNode::Opt { .. } => "opt",
        TypeNode::Vec { .. } => "vec",
        TypeNode::Record { .. } => "record",
        TypeNode::Variant { .. } => "variant",
        TypeNode::Func { .. } => "func",
        TypeNode::Service { .. } => "service",
        TypeNode::Class { .. } => "class",
    }
}

fn host_value_kind(value: &HostValue) -> &'static str {
    match value {
        HostValue::Null => "null",
        HostValue::Bool { .. } => "bool",
        HostValue::Nat { .. } => "nat",
        HostValue::Int { .. } => "int",
        HostValue::Nat8 { .. } => "nat8",
        HostValue::Nat16 { .. } => "nat16",
        HostValue::Nat32 { .. } => "nat32",
        HostValue::Nat64 { .. } => "nat64",
        HostValue::Int8 { .. } => "int8",
        HostValue::Int16 { .. } => "int16",
        HostValue::Int32 { .. } => "int32",
        HostValue::Int64 { .. } => "int64",
        HostValue::Float32 { .. } => "float32",
        HostValue::Float64 { .. } => "float64",
        HostValue::Text { .. } => "text",
        HostValue::Reserved => "reserved",
        HostValue::Principal { .. } => "principal",
        HostValue::Opt { .. } => "opt",
        HostValue::Vec { .. } => "vec",
        HostValue::Record { .. } => "record",
        HostValue::Variant { .. } => "variant",
        HostValue::Service { .. } => "service",
        HostValue::Func { .. } => "func",
    }
}

impl Contract {
    pub fn bind_type(
        &self,
        type_ref: TypeRef,
    ) -> Result<ContractTypeRef, HostValueValidationError> {
        if type_ref as usize >= self.types().len() {
            return Err(single(
                "value_type_ref_out_of_bounds",
                "$",
                format!("type reference {type_ref} is outside the Contract"),
            ));
        }
        Ok(ContractTypeRef {
            contract_id: self.contract_id().to_string(),
            type_ref,
        })
    }

    pub fn bind_method(
        &self,
        method: impl Into<String>,
    ) -> Result<ContractMethodRef, HostValueValidationError> {
        let method = method.into();
        let service = match self.actor() {
            Some(crate::model::Actor::Service { service }) => *service,
            Some(crate::model::Actor::Class { class }) => match &self.types()[*class as usize] {
                TypeNode::Class { service, .. } => *service,
                _ => unreachable!("validated class actor targets a class"),
            },
            None => {
                return Err(single(
                    "actorless_contract",
                    "$",
                    "an actorless Contract has no methods",
                ));
            }
        };
        match &self.types()[service as usize] {
            TypeNode::Service { methods } if methods.iter().any(|entry| entry.name == method) => {
                Ok(ContractMethodRef {
                    contract_id: self.contract_id().to_string(),
                    method,
                })
            }
            _ => Err(single(
                "unknown_method",
                "$",
                format!("method {method:?} does not exist in the actor service"),
            )),
        }
    }
}
