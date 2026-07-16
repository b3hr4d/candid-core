use crate::diagnostics::{CompileError, DiagnosticPhase};
use crate::limits::Limits;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct SourceId(String);

impl SourceId {
    pub fn parse(value: impl AsRef<str>) -> Result<Self, ResolveError> {
        let value = value.as_ref();
        if let Some((scheme, path)) = value.split_once(":/") {
            validate_scheme(scheme)?;
            let path = normalize_path(None, path)?;
            Ok(Self(format!("{scheme}:/{path}")))
        } else {
            let path = normalize_path(None, value)?;
            Ok(Self(format!("memory:/{path}")))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn scheme(&self) -> &str {
        self.0
            .split_once(":/")
            .map(|(scheme, _)| scheme)
            .unwrap_or("")
    }

    pub fn path(&self) -> &str {
        self.0
            .split_once(":/")
            .map(|(_, path)| path.trim_start_matches('/'))
            .unwrap_or("")
    }

    fn with_scheme(scheme: &str, path: String) -> Self {
        Self(format!("{scheme}:/{path}"))
    }
}

impl<'de> Deserialize<'de> for SourceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl std::str::FromStr for SourceId {
    type Err = ResolveError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl TryFrom<&str> for SourceId {
    type Error = ResolveError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSource {
    pub id: SourceId,
    pub source: String,
    pub digest: String,
}

impl ResolvedSource {
    fn new(id: SourceId, source: String) -> Self {
        let digest = format!("sha256:{}", hex::encode(Sha256::digest(source.as_bytes())));
        Self { id, source, digest }
    }

    pub fn verify(&self) -> Result<(), ResolveError> {
        let expected = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(self.source.as_bytes()))
        );
        if self.digest != expected {
            return Err(ResolveError::new(
                "did_source_digest_mismatch",
                format!(
                    "source {:?} declared digest {}, expected {expected}",
                    self.id.as_str(),
                    self.digest
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveError {
    pub code: String,
    pub message: String,
    pub resource_limit: Option<crate::ResourceLimitInfo>,
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ResolveError {}

impl ResolveError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            resource_limit: None,
        }
    }

    fn resource_limit(resource: &str, limit: usize, observed: usize, message: String) -> Self {
        Self {
            code: "resource_limit_exceeded".to_string(),
            message,
            resource_limit: Some(crate::ResourceLimitInfo {
                resource: resource.to_string(),
                limit,
                observed,
            }),
        }
    }

    pub(crate) fn into_compile_error(self) -> CompileError {
        match self.resource_limit {
            Some(info) => CompileError::resource_limit(
                &info.resource,
                info.limit,
                info.observed,
                self.message,
            ),
            None => CompileError::single(self.code, DiagnosticPhase::Load, self.message),
        }
    }
}

pub trait SourceResolver {
    fn identify(&self, from: Option<&SourceId>, import: &str) -> Result<SourceId, ResolveError>;

    fn load(&self, id: &SourceId, limits: &Limits) -> Result<ResolvedSource, ResolveError>;

    fn resolve(
        &self,
        from: Option<&SourceId>,
        import: &str,
        limits: &Limits,
    ) -> Result<ResolvedSource, ResolveError> {
        let id = self.identify(from, import)?;
        self.load(&id, limits)
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemoryResolver {
    sources: BTreeMap<SourceId, String>,
}

impl MemoryResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &mut self,
        id: impl AsRef<str>,
        source: impl Into<String>,
    ) -> Result<(), ResolveError> {
        let id = SourceId::parse(id)?;
        if id.scheme() != "memory" {
            return Err(ResolveError::new(
                "did_source_scheme_mismatch",
                "MemoryResolver source IDs must use memory:/",
            ));
        }
        self.sources.insert(id, source.into());
        Ok(())
    }

    pub fn with_source(
        mut self,
        id: impl AsRef<str>,
        source: impl Into<String>,
    ) -> Result<Self, ResolveError> {
        self.insert(id, source)?;
        Ok(self)
    }
}

impl SourceResolver for MemoryResolver {
    fn identify(&self, from: Option<&SourceId>, import: &str) -> Result<SourceId, ResolveError> {
        if from.is_some_and(|from| from.scheme() != "memory") {
            return Err(ResolveError::new(
                "did_source_scheme_mismatch",
                "MemoryResolver can only resolve memory:/ sources",
            ));
        }
        let id = if from.is_none() && import.contains(":/") {
            let id = SourceId::parse(import)?;
            if id.scheme() != "memory" {
                return Err(ResolveError::new(
                    "did_source_scheme_mismatch",
                    "MemoryResolver entry IDs must use memory:/",
                ));
            }
            id
        } else {
            SourceId::with_scheme("memory", normalize_path(from, import)?)
        };
        Ok(id)
    }

    fn load(&self, id: &SourceId, limits: &Limits) -> Result<ResolvedSource, ResolveError> {
        if id.scheme() != "memory" {
            return Err(ResolveError::new(
                "did_source_scheme_mismatch",
                "MemoryResolver can only load memory:/ sources",
            ));
        }
        let source = self.sources.get(id).cloned().ok_or_else(|| {
            ResolveError::new(
                "did_source_not_found",
                format!(
                    "source {:?} is not present in the memory bundle",
                    id.as_str()
                ),
            )
        })?;
        check_source_size(id, &source, limits)?;
        Ok(ResolvedSource::new(id.clone(), source))
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceResolver {
    root: PathBuf,
}

impl WorkspaceResolver {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, ResolveError> {
        let root = fs::canonicalize(root.as_ref()).map_err(|error| {
            ResolveError::new(
                "did_workspace_root_error",
                format!(
                    "cannot open workspace root {}: {error}",
                    root.as_ref().display()
                ),
            )
        })?;
        if !root.is_dir() {
            return Err(ResolveError::new(
                "did_workspace_root_error",
                format!("workspace root {} is not a directory", root.display()),
            ));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl SourceResolver for WorkspaceResolver {
    fn identify(&self, from: Option<&SourceId>, import: &str) -> Result<SourceId, ResolveError> {
        if from.is_some_and(|from| from.scheme() != "workspace") {
            return Err(ResolveError::new(
                "did_source_scheme_mismatch",
                "WorkspaceResolver can only resolve workspace:/ sources",
            ));
        }
        let id = if from.is_none() && import.contains(":/") {
            let id = SourceId::parse(import)?;
            if id.scheme() != "workspace" {
                return Err(ResolveError::new(
                    "did_source_scheme_mismatch",
                    "WorkspaceResolver entry IDs must use workspace:/",
                ));
            }
            id
        } else {
            SourceId::with_scheme("workspace", normalize_path(from, import)?)
        };
        Ok(id)
    }

    fn load(&self, id: &SourceId, limits: &Limits) -> Result<ResolvedSource, ResolveError> {
        if id.scheme() != "workspace" {
            return Err(ResolveError::new(
                "did_source_scheme_mismatch",
                "WorkspaceResolver can only load workspace:/ sources",
            ));
        }
        let mut candidate = self.root.clone();
        for segment in id.path().split('/') {
            candidate.push(segment);
        }
        let canonical = fs::canonicalize(&candidate).map_err(|error| {
            ResolveError::new(
                "did_file_read_error",
                format!("cannot read source {:?}: {error}", id.as_str()),
            )
        })?;
        if !canonical.starts_with(&self.root) {
            return Err(ResolveError::new(
                "did_import_outside_workspace",
                format!(
                    "source {:?} resolves outside the authorized workspace",
                    id.as_str()
                ),
            ));
        }
        let source = fs::read_to_string(&canonical).map_err(|error| {
            ResolveError::new(
                "did_file_read_error",
                format!("cannot read source {:?}: {error}", id.as_str()),
            )
        })?;
        check_source_size(id, &source, limits)?;
        Ok(ResolvedSource::new(id.clone(), source))
    }
}

fn normalize_path(from: Option<&SourceId>, import: &str) -> Result<String, ResolveError> {
    if import.is_empty() {
        return Err(ResolveError::new(
            "did_invalid_source_id",
            "source IDs and imports must not be empty",
        ));
    }
    if import.starts_with('/') {
        return Err(ResolveError::new(
            "did_absolute_import_forbidden",
            format!("absolute import {import:?} is not permitted"),
        ));
    }
    if import.contains('\\') {
        return Err(ResolveError::new(
            "did_invalid_source_id",
            format!("backslashes are not permitted in logical source path {import:?}"),
        ));
    }
    if import.chars().any(char::is_control) {
        return Err(ResolveError::new(
            "did_invalid_source_id",
            format!("control characters are not permitted in logical source path {import:?}"),
        ));
    }
    if import.split('/').any(str::is_empty) {
        return Err(ResolveError::new(
            "did_invalid_source_id",
            format!("empty segments are not permitted in logical source path {import:?}"),
        ));
    }

    let mut components = Vec::<String>::new();
    if let Some(from) = from {
        if let Some((parent, _)) = from.path().rsplit_once('/') {
            components.extend(parent.split('/').map(str::to_owned));
        }
    }
    for component in import.split('/') {
        match component {
            "." => {}
            ".." => {
                if components.pop().is_none() {
                    return Err(ResolveError::new(
                        "did_import_outside_workspace",
                        format!("import {import:?} escapes the authorized source root"),
                    ));
                }
            }
            value if value.contains(':') => {
                return Err(ResolveError::new(
                    "did_invalid_source_id",
                    format!("colons are not permitted in logical source path {import:?}"),
                ));
            }
            value => components.push(value.to_owned()),
        }
    }
    if components.is_empty() {
        return Err(ResolveError::new(
            "did_invalid_source_id",
            format!("source ID {import:?} does not name a file"),
        ));
    }
    Ok(components.join("/"))
}

fn validate_scheme(scheme: &str) -> Result<(), ResolveError> {
    let mut bytes = scheme.bytes();
    if scheme.len() < 2
        || !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        || !bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(ResolveError::new(
            "did_invalid_source_id",
            format!("invalid logical source scheme {scheme:?}"),
        ));
    }
    Ok(())
}

fn check_source_size(id: &SourceId, source: &str, limits: &Limits) -> Result<(), ResolveError> {
    if source.len() > limits.max_source_bytes {
        return Err(ResolveError::resource_limit(
            "source_bytes",
            limits.max_source_bytes,
            source.len(),
            format!(
                "source {:?} uses {} bytes; limit is {}",
                id.as_str(),
                source.len(),
                limits.max_source_bytes
            ),
        ));
    }
    Ok(())
}
