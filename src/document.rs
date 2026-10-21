//! `Document`, `YamlFile`, `Directive`, and the results of validation.

use std::path::PathBuf;
use std::str::FromStr;

use pyo3::exceptions::{PyIndexError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString, PyTuple};
use rowan::ast::AstNode;
use yaml_edit::anchor_resolution::{AnchorRegistry, DocumentMergedExt};
use yaml_edit::validator::{Validator, ValidatorConfig, Violation};
use yaml_edit::{Directive, Document, Mapping, SchemaValidator, YamlFile, YamlNode};

use crate::collections::{
    self, deprecated, field_order, place, resolve_index, Placement, PyKeyIterator, PyMapping,
    PyMergedMapping, PySequence,
};
use crate::convert::{call_with_key_value, call_with_value, to_python, ValueArg};
use crate::errors::{file_error, io_error, resolve_error, yaml_error};
use crate::nodes::{PyComment, PyNode, PyScalar, PyTaggedNode};
use crate::{compare, line_column, span};

/// The root node of a document, or `None` if it is empty.
fn root_node(doc: &Document) -> Option<YamlNode> {
    doc.as_mapping()
        .map(YamlNode::Mapping)
        .or_else(|| doc.as_sequence().map(YamlNode::Sequence))
        .or_else(|| doc.as_tagged().map(YamlNode::TaggedNode))
        .or_else(|| doc.as_scalar().map(YamlNode::Scalar))
}

/// Convert `CamelCase` to `snake_case`.
fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A problem found by `Document.validate()` or `YamlFile.validate()`.
#[pyclass(name = "Violation", frozen, module = "yaml_edit._yaml_edit")]
pub(crate) struct PyViolation {
    /// A description of the problem.
    #[pyo3(get)]
    message: String,
    /// The 1-based "line:column" where the problem starts, if known.
    #[pyo3(get)]
    location: Option<String>,
    /// The `(start, end)` byte offsets of the problem, if known.
    #[pyo3(get)]
    byte_range: Option<(u32, u32)>,
    /// "error" for invalid YAML, "warning" for discouraged constructs.
    #[pyo3(get)]
    severity: String,
    /// The rule violated, such as "duplicate_keys" or "invalid_indentation".
    #[pyo3(get)]
    rule: String,
}

impl From<Violation> for PyViolation {
    fn from(violation: Violation) -> Self {
        PyViolation {
            message: violation.message,
            location: violation.location,
            byte_range: violation.text_range.map(span),
            severity: snake_case(&format!("{:?}", violation.severity)),
            rule: snake_case(&format!("{:?}", violation.rule)),
        }
    }
}

#[pymethods]
impl PyViolation {
    fn __repr__(&self) -> String {
        format!(
            "<yaml_edit.Violation {} {} at {}: {:?}>",
            self.severity,
            self.rule,
            self.location.as_deref().unwrap_or("?"),
            self.message
        )
    }
}

/// A scalar that `Document.validate_schema()` found the schema does not allow.
#[pyclass(name = "SchemaViolation", frozen, module = "yaml_edit._yaml_edit")]
pub(crate) struct PySchemaViolation {
    /// The path of the offending value, such as "server.port".
    #[pyo3(get)]
    path: String,
    /// A description of the problem.
    #[pyo3(get)]
    message: String,
    /// The name of the schema.
    #[pyo3(get)]
    schema: String,
}

#[pymethods]
impl PySchemaViolation {
    fn __repr__(&self) -> String {
        format!(
            "<yaml_edit.SchemaViolation {}: {:?}>",
            self.path, self.message
        )
    }
}

fn validator(
    duplicate_keys: bool,
    indentation: bool,
    tabs: bool,
    document_markers: bool,
    anchors: bool,
) -> Validator {
    Validator::with_config(ValidatorConfig {
        check_duplicate_keys: duplicate_keys,
        check_indentation: indentation,
        check_tabs: tabs,
        check_document_markers: document_markers,
        check_anchors: anchors,
    })
}

/// A single YAML document.
///
/// For a document whose root is a mapping, `Document` has the same `dict`
/// interface as `Mapping`.
#[pyclass(name = "Document", unsendable, module = "yaml_edit._yaml_edit")]
pub(crate) struct PyDocument {
    pub(crate) inner: Document,
}

impl PyDocument {
    fn root_mapping(&self) -> PyResult<Mapping> {
        self.inner
            .as_mapping()
            .ok_or_else(|| PyTypeError::new_err("document root is not a mapping"))
    }
}

#[pymethods]
impl PyDocument {
    /// Create an empty document.
    #[new]
    fn new() -> Self {
        PyDocument {
            inner: Document::new(),
        }
    }

    /// Parse a document from a YAML string.
    ///
    /// Raises `ParseError` if the text is not valid YAML.
    #[staticmethod]
    fn parse(text: &str) -> PyResult<Self> {
        Document::from_str(text)
            .map(|inner| PyDocument { inner })
            .map_err(yaml_error)
    }

    /// Parse a document, recovering from syntax errors instead of raising.
    ///
    /// Returns the document and a list of error messages, empty if the text
    /// parsed cleanly.
    #[staticmethod]
    fn parse_relaxed(text: &str) -> (Self, Vec<String>) {
        let (inner, errors) = Document::from_str_relaxed(text);
        (PyDocument { inner }, errors)
    }

    /// Parse a document from a file.
    #[staticmethod]
    fn from_file(path: PathBuf) -> PyResult<Self> {
        Document::from_file(&path)
            .map(|inner| PyDocument { inner })
            .map_err(|e| file_error(e, &path))
    }

    /// Write the document to a file.
    fn to_file(&self, path: PathBuf) -> PyResult<()> {
        self.inner.to_file(&path).map_err(|e| file_error(e, &path))
    }

    /// The comments in this document, in source order.
    fn comments(&self) -> Vec<PyComment> {
        self.inner
            .comments()
            .map(|inner| PyComment { inner })
            .collect()
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The root node, or `None` if the document is empty.
    fn root(&self) -> Option<PyNode> {
        root_node(&self.inner).map(|inner| PyNode { inner })
    }

    /// The document root as a mapping, or `None` if it is not a mapping.
    fn as_mapping(&self) -> Option<PyMapping> {
        self.inner.as_mapping().map(|inner| PyMapping { inner })
    }

    /// The document root as a sequence, or `None` if it is not a sequence.
    fn as_sequence(&self) -> Option<PySequence> {
        self.inner.as_sequence().map(|inner| PySequence { inner })
    }

    /// The document root as a scalar, or `None` if it is not a scalar.
    fn as_scalar(&self) -> Option<PyScalar> {
        self.inner.as_scalar().map(|inner| PyScalar { inner })
    }

    /// The document root as a tagged node, or `None` if it is not tagged.
    fn as_tagged(&self) -> Option<PyTaggedNode> {
        self.inner.as_tagged().map(|inner| PyTaggedNode { inner })
    }

    /// A merged view of the root mapping, resolving aliases and merge keys
    /// (`<<`), or `None` if the root is not a mapping.
    ///
    /// The anchor registry is built from the whole document, so aliases
    /// pointing at anchors defined elsewhere resolve correctly. Raises
    /// `ResolveError` if an alias or merge key cannot be resolved.
    fn merged(&self) -> PyResult<Option<PyMergedMapping>> {
        let view = self.inner.merged().map_err(resolve_error)?;
        Ok(view.map(|view| PyMergedMapping {
            base: view.as_mapping().base().clone(),
            registry: view.registry().clone(),
        }))
    }

    /// The names of the anchors (`&name`) defined in this document.
    fn anchors(&self) -> Vec<String> {
        AnchorRegistry::from_document(&self.inner)
            .anchor_names()
            .map(str::to_string)
            .collect()
    }

    /// Convert to plain Python objects, resolving aliases and merge keys.
    ///
    /// An empty document is `None`.
    fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match root_node(&self.inner) {
            Some(root) => to_python(py, &root),
            None => Ok(py.None()),
        }
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn contains_key(&self, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        collections::contains(&self.inner, key)
    }

    fn __contains__(&self, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        collections::contains(&self.inner, key)
    }

    /// The value node for a top-level key, or `default` if absent.
    #[pyo3(signature = (key, default=None))]
    fn get(
        &self,
        py: Python<'_>,
        key: &Bound<'_, PyAny>,
        default: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        collections::get_or(py, &self.inner, key, default)
    }

    fn __getitem__(&self, key: &Bound<'_, PyAny>) -> PyResult<PyNode> {
        collections::getitem(&self.inner, key)
    }

    /// The value of a top-level key as a string, or `None`.
    fn get_string(&self, key: &Bound<'_, PyAny>) -> PyResult<Option<String>> {
        let key = ValueArg::extract(key)?;
        Ok(call_with_value!(&key, |k| self.inner.get_string(k)))
    }

    /// The value for a top-level key if it is a mapping, or `None`.
    fn get_mapping(&self, key: &Bound<'_, PyAny>) -> PyResult<Option<PyMapping>> {
        let key = ValueArg::extract(key)?;
        Ok(call_with_value!(&key, |k| self.inner.get_mapping(k)).map(|inner| PyMapping { inner }))
    }

    /// The value for a top-level key if it is a sequence, or `None`.
    fn get_sequence(&self, key: &Bound<'_, PyAny>) -> PyResult<Option<PySequence>> {
        let key = ValueArg::extract(key)?;
        Ok(
            call_with_value!(&key, |k| self.inner.get_sequence(k))
                .map(|inner| PySequence { inner }),
        )
    }

    /// Set a top-level key, creating a root mapping if the document is empty.
    ///
    /// With `field_order` (a list of keys), a new key is placed after the keys
    /// that precede it in that order. Raises `TypeError` if the root is not a
    /// mapping.
    #[pyo3(signature = (key, value, *, field_order=None))]
    fn set(
        &self,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
        field_order: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        collections::set(&self.inner, key, value, field_order)
    }

    fn __setitem__(&self, key: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        collections::set(&self.inner, key, value, None)
    }

    /// Remove a top-level key, returning `True` if it was present.
    fn remove(&self, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        collections::remove(&self.inner, key)
    }

    fn __delitem__(&self, key: &Bound<'_, PyAny>) -> PyResult<()> {
        collections::delitem(&self.inner, key)
    }

    /// Remove a top-level key and return its value node, or `default`.
    ///
    /// Raises `KeyError` if `key` is absent and no default is given.
    #[pyo3(signature = (key, *default))]
    fn pop(
        &self,
        py: Python<'_>,
        key: &Bound<'_, PyAny>,
        default: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        collections::pop(py, &self.inner, key, default)
    }

    /// Remove and return the last top-level `(key, value)` pair.
    fn popitem(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, Py<PyAny>)> {
        collections::popitem(py, &self.inner)
    }

    /// Return the value node for `key`, first setting it to `default` if absent.
    #[pyo3(signature = (key, default=None))]
    fn setdefault(
        &self,
        py: Python<'_>,
        key: &Bound<'_, PyAny>,
        default: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyNode> {
        let none = py.None().into_bound(py);
        collections::setdefault(&self.inner, key, default.unwrap_or(&none))
    }

    /// Set keys from a mapping or iterable of pairs, and keyword arguments.
    #[pyo3(signature = (other=None, **kwargs))]
    fn update(
        &self,
        other: Option<&Bound<'_, PyAny>>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        collections::update(&self.inner, other, kwargs)
    }

    fn rename_key(&self, old_key: &Bound<'_, PyAny>, new_key: &Bound<'_, PyAny>) -> PyResult<bool> {
        collections::rename_key(&self.inner, old_key, new_key)
    }

    /// Remove every top-level key.
    fn clear(&self) {
        if let Some(mapping) = self.inner.as_mapping() {
            mapping.clear();
        }
    }

    /// The top-level keys, in document order.
    fn keys(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        collections::keys(py, &self.inner)
    }

    /// The top-level value nodes, in document order.
    fn values(&self) -> Vec<PyNode> {
        collections::values(&self.inner)
    }

    /// Top-level `(key, value node)` pairs, in document order.
    fn items(&self, py: Python<'_>) -> PyResult<Vec<(Py<PyAny>, PyNode)>> {
        collections::items(py, &self.inner)
    }

    fn __iter__(&self, py: Python<'_>) -> PyResult<PyKeyIterator> {
        Ok(PyKeyIterator {
            keys: collections::keys(py, &self.inner)?.into_iter(),
        })
    }

    /// Look up a dotted path such as "server.ports[0]", or `None` if absent.
    ///
    /// Raises `PathError` for a malformed path or one that descends into a
    /// scalar. `alias_policy` is "follow" (the default), "refuse" or "expand".
    #[pyo3(signature = (path, alias_policy="follow"))]
    fn get_path(&self, path: &str, alias_policy: &str) -> PyResult<Option<PyNode>> {
        crate::get_path(&self.inner, path, alias_policy)
    }

    /// Set a dotted path, creating intermediate mappings as needed.
    ///
    /// Raises `PathError` if the path cannot be set. Writing through an alias
    /// is refused by default; pass `alias_policy="follow"` to modify the
    /// anchored node, or "expand" to replace the alias with a copy first.
    #[pyo3(signature = (path, value, alias_policy="refuse"))]
    fn set_path(&self, path: &str, value: &Bound<'_, PyAny>, alias_policy: &str) -> PyResult<()> {
        crate::set_path(&self.inner, path, value, alias_policy)
    }

    /// Remove a dotted path, returning `True` if it was present.
    #[pyo3(signature = (path, alias_policy="refuse"))]
    fn remove_path(&self, path: &str, alias_policy: &str) -> PyResult<bool> {
        crate::remove_path(&self.inner, path, alias_policy)
    }

    /// Insert `key` immediately after `after_key`.
    ///
    /// An existing `key` is updated in place rather than moved. Returns
    /// `False` if `after_key` is not present. Raises `TypeError` if the root
    /// is not a mapping.
    fn insert_after(
        &self,
        after_key: &Bound<'_, PyAny>,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        place(
            &self.root_mapping()?,
            Placement::InsertAfter,
            after_key,
            key,
            value,
        )
    }

    /// Insert `key` immediately before `before_key`.
    ///
    /// An existing `key` is updated in place rather than moved. Returns
    /// `False` if `before_key` is not present. Raises `TypeError` if the root
    /// is not a mapping.
    fn insert_before(
        &self,
        before_key: &Bound<'_, PyAny>,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        place(
            &self.root_mapping()?,
            Placement::InsertBefore,
            before_key,
            key,
            value,
        )
    }

    /// Set `key` and place it immediately after `after_key`, moving it if it
    /// already exists. Returns `False` if `after_key` is not present.
    ///
    /// Raises `TypeError` if the root is not a mapping.
    fn move_after(
        &self,
        after_key: &Bound<'_, PyAny>,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        place(
            &self.root_mapping()?,
            Placement::MoveAfter,
            after_key,
            key,
            value,
        )
    }

    /// Set `key` and place it immediately before `before_key`, moving it if
    /// it already exists. Returns `False` if `before_key` is not present.
    ///
    /// Raises `TypeError` if the root is not a mapping.
    fn move_before(
        &self,
        before_key: &Bound<'_, PyAny>,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        place(
            &self.root_mapping()?,
            Placement::MoveBefore,
            before_key,
            key,
            value,
        )
    }

    /// Insert `key` at position `index`, or at the end if `index` is past it.
    ///
    /// An existing `key` is updated in place rather than moved. Raises
    /// `TypeError` if the root is not a mapping.
    fn insert_at_index(
        &self,
        index: usize,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let key = ValueArg::extract(key)?;
        let value = ValueArg::extract(value)?;
        if call_with_key_value!(&key, &value, |k, v| self.inner.insert_at_index(index, k, v)) {
            Ok(())
        } else {
            Err(PyTypeError::new_err("document root is not a mapping"))
        }
    }

    /// Reorder entries so the keys in `order` come first, in that order.
    ///
    /// Keys not in `order` follow, in their original relative order. Raises
    /// `TypeError` if the root is not a mapping.
    fn reorder_fields(&self, order: &Bound<'_, PyAny>) -> PyResult<()> {
        if self.inner.reorder_fields(field_order(order)?) {
            Ok(())
        } else {
            Err(PyTypeError::new_err("document root is not a mapping"))
        }
    }

    /// Check the document against the YAML 1.2 specification.
    ///
    /// Returns the problems found, empty if there are none. The keyword
    /// arguments turn individual checks off.
    #[pyo3(signature = (*, duplicate_keys=true, indentation=true, tabs=true, document_markers=true, anchors=true))]
    fn validate(
        &self,
        duplicate_keys: bool,
        indentation: bool,
        tabs: bool,
        document_markers: bool,
        anchors: bool,
    ) -> Vec<PyViolation> {
        validator(duplicate_keys, indentation, tabs, document_markers, anchors)
            .validate(&self.inner)
            .into_iter()
            .map(PyViolation::from)
            .collect()
    }

    /// Check that every scalar has a type `schema` allows: "failsafe" (only
    /// strings), "json" or "core".
    ///
    /// Returns the scalars that do not, empty if all do. Without `strict`, a
    /// value that could be read as an allowed type is accepted, so `1`
    /// passes "failsafe" as the string "1"; `strict=True` reports it.
    #[pyo3(signature = (schema="core", *, strict=false))]
    fn validate_schema(&self, schema: &str, strict: bool) -> PyResult<Vec<PySchemaViolation>> {
        let validator = match schema {
            "failsafe" => SchemaValidator::failsafe(),
            "json" => SchemaValidator::json(),
            "core" => SchemaValidator::core(),
            _ => {
                return Err(PyValueError::new_err(format!(
                    "unknown schema {schema:?}; expected \"failsafe\", \"json\" or \"core\""
                )))
            }
        };
        let validator = if strict {
            validator.strict()
        } else {
            validator
        };
        Ok(match self.inner.validate_schema(&validator) {
            Ok(()) => Vec::new(),
            Err(errors) => errors
                .into_iter()
                .map(|error| PySchemaViolation {
                    message: error.message(),
                    path: error.path,
                    schema: error.schema_name,
                })
                .collect(),
        })
    }

    /// Rewrite YAML 1.1 scalars so YAML 1.2 readers see the same values, such
    /// as `yes` to `true` and `0755` to `0o755`.
    ///
    /// Returns the `(before, after)` text of each rewritten scalar. Run it
    /// only once per document.
    fn migrate_to_yaml_1_2(&self) -> Vec<(String, String)> {
        yaml_edit::migrate::to_yaml_1_2(&self.inner)
            .into_iter()
            .map(|rewrite| (rewrite.before, rewrite.after))
            .collect()
    }

    /// The `(start, end)` byte offsets of this document in the source text.
    fn byte_range(&self) -> (u32, u32) {
        span(self.inner.byte_range())
    }

    /// The 1-based `(line, column)` where this document starts.
    fn start_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().start)
    }

    /// The 1-based `(line, column)` where this document ends.
    fn end_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().end)
    }

    /// An independent copy of the document.
    fn __copy__(&self) -> Self {
        PyDocument {
            inner: Document::from_str_relaxed(&self.inner.to_string()).0,
        }
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.__copy__()
    }

    /// Compare by YAML value, ignoring formatting.
    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(other) = other.extract::<PyRef<'_, PyDocument>>() {
            return Ok(
                pyo3::types::PyBool::new(py, yaml_edit::yaml_eq(&self.inner, &other.inner))
                    .to_owned()
                    .into_any()
                    .unbind(),
            );
        }
        compare(py, &self.inner, other)
    }

    /// Serialize the document back to a string, preserving formatting.
    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Document {:?}>", self.__str__())
    }
}

/// A directive line such as `%YAML 1.2` or `%TAG ! tag:example.com,2000:`.
///
/// Compares equal to its text, so `directive == "%YAML 1.2"` works.
#[pyclass(name = "Directive", unsendable, module = "yaml_edit._yaml_edit")]
pub(crate) struct PyDirective {
    inner: Directive,
}

#[pymethods]
impl PyDirective {
    /// A `%YAML <version>` directive.
    #[staticmethod]
    fn yaml_version(version: &str) -> Self {
        PyDirective {
            inner: Directive::new_yaml_version(version),
        }
    }

    /// A `%TAG <handle> <prefix>` directive.
    #[staticmethod]
    fn tag(handle: &str, prefix: &str) -> Self {
        PyDirective {
            inner: Directive::new_tag(handle, prefix),
        }
    }

    /// The full directive text, such as "%YAML 1.2".
    fn text(&self) -> String {
        self.inner.text()
    }

    /// The directive name, such as "YAML" or "TAG".
    fn name(&self) -> Option<String> {
        self.inner.name()
    }

    /// The directive's parameters, such as "1.2".
    fn value(&self) -> Option<String> {
        self.inner.value()
    }

    fn is_yaml_version(&self) -> bool {
        self.inner.is_yaml_version()
    }

    fn is_tag(&self) -> bool {
        self.inner.is_tag()
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> PyResult<Option<bool>> {
        if let Ok(other) = other.extract::<PyRef<'_, PyDirective>>() {
            return Ok(Some(self.inner.text() == other.inner.text()));
        }
        if let Ok(other) = other.extract::<String>() {
            return Ok(Some(self.inner.text() == other));
        }
        Ok(None)
    }

    fn __hash__(&self, py: Python<'_>) -> PyResult<isize> {
        PyString::new(py, &self.inner.text()).hash()
    }

    fn __str__(&self) -> String {
        self.inner.text()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Directive {:?}>", self.inner.text())
    }
}

/// A YAML stream, which may hold multiple documents and directives.
///
/// Behaves as a sequence of `Document` objects.
#[pyclass(
    name = "YamlFile",
    sequence,
    unsendable,
    module = "yaml_edit._yaml_edit"
)]
pub(crate) struct PyYamlFile {
    inner: YamlFile,
}

#[pymethods]
impl PyYamlFile {
    /// Create an empty YAML stream.
    #[new]
    fn new() -> Self {
        PyYamlFile {
            inner: YamlFile::new(),
        }
    }

    /// Parse a YAML stream from a string.
    ///
    /// Raises `ParseError` if the text is not valid YAML.
    #[staticmethod]
    fn parse(text: &str) -> PyResult<Self> {
        YamlFile::from_str(text)
            .map(|inner| PyYamlFile { inner })
            .map_err(yaml_error)
    }

    /// Parse a YAML stream, recovering from syntax errors instead of raising.
    ///
    /// Returns the stream and a list of error messages, empty if the text
    /// parsed cleanly.
    #[staticmethod]
    fn parse_relaxed(text: &str) -> (Self, Vec<String>) {
        let (inner, errors) = YamlFile::from_str_relaxed(text);
        (PyYamlFile { inner }, errors)
    }

    /// Parse a YAML stream from a file.
    #[staticmethod]
    fn from_file(path: PathBuf) -> PyResult<Self> {
        YamlFile::from_path(&path)
            .map(|inner| PyYamlFile { inner })
            .map_err(|e| file_error(e, &path))
    }

    /// Write the stream to a file.
    fn to_file(&self, path: PathBuf) -> PyResult<()> {
        std::fs::write(&path, self.inner.to_string()).map_err(|e| io_error(e, &path))
    }

    fn __len__(&self) -> usize {
        self.inner.documents().count()
    }

    fn is_empty(&self) -> bool {
        self.inner.documents().next().is_none()
    }

    /// The document at `index`; negative indices count from the end.
    fn __getitem__(&self, index: isize) -> PyResult<PyDocument> {
        let documents: Vec<Document> = self.inner.documents().collect();
        resolve_index(index, documents.len())
            .and_then(|i| documents.into_iter().nth(i))
            .map(|inner| PyDocument { inner })
            .ok_or_else(|| PyIndexError::new_err("document index out of range"))
    }

    fn __iter__(&self) -> PyDocumentIterator {
        PyDocumentIterator {
            documents: self.documents().into_iter(),
        }
    }

    /// The documents in this stream.
    fn documents(&self) -> Vec<PyDocument> {
        self.inner
            .documents()
            .map(|inner| PyDocument { inner })
            .collect()
    }

    /// The first document, or `None` if the stream is empty.
    fn document(&self) -> Option<PyDocument> {
        self.inner.document().map(|inner| PyDocument { inner })
    }

    /// Return the first document, creating one if the stream is empty.
    fn ensure_document(&self) -> PyDocument {
        PyDocument {
            inner: self.inner.ensure_document(),
        }
    }

    /// Append a document to the stream.
    fn append(&self, document: &PyDocument) {
        self.inner.push_document(document.inner.clone());
    }

    /// Deprecated alias of `append`.
    fn push_document(&self, py: Python<'_>, document: &PyDocument) -> PyResult<()> {
        deprecated(py, c"YamlFile.push_document() is deprecated; use append()")?;
        self.inner.push_document(document.inner.clone());
        Ok(())
    }

    /// The directives (such as `%YAML 1.2`) in this stream.
    fn directives(&self) -> Vec<PyDirective> {
        self.inner
            .directives()
            .map(|inner| PyDirective { inner })
            .collect()
    }

    /// Add a directive, given as a `Directive` or as text like "%YAML 1.2".
    fn add_directive(&self, directive: &Bound<'_, PyAny>) -> PyResult<()> {
        let text = match directive.extract::<PyRef<'_, PyDirective>>() {
            Ok(directive) => directive.inner.text(),
            Err(_) => directive.extract::<String>()?,
        };
        self.inner.add_directive(&text);
        Ok(())
    }

    /// The comments in this stream, in source order.
    fn comments(&self) -> Vec<PyComment> {
        self.inner
            .comments()
            .map(|inner| PyComment { inner })
            .collect()
    }

    /// Check the whole stream, directives included, against the YAML 1.2
    /// specification. See `Document.validate`.
    #[pyo3(signature = (*, duplicate_keys=true, indentation=true, tabs=true, document_markers=true, anchors=true))]
    fn validate(
        &self,
        duplicate_keys: bool,
        indentation: bool,
        tabs: bool,
        document_markers: bool,
        anchors: bool,
    ) -> Vec<PyViolation> {
        validator(duplicate_keys, indentation, tabs, document_markers, anchors)
            .validate_syntax(self.inner.syntax())
            .into_iter()
            .map(PyViolation::from)
            .collect()
    }

    /// An independent copy of the stream.
    fn __copy__(&self) -> Self {
        PyYamlFile {
            inner: self.inner.snapshot(),
        }
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.__copy__()
    }

    /// Serialize the whole stream back to a string, preserving formatting.
    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!(
            "<yaml_edit.YamlFile docs={}>",
            self.inner.documents().count()
        )
    }
}

/// Iterator over a stream's documents, returned by `YamlFile.__iter__`.
#[pyclass(unsendable, module = "yaml_edit._yaml_edit")]
pub(crate) struct PyDocumentIterator {
    documents: std::vec::IntoIter<PyDocument>,
}

#[pymethods]
impl PyDocumentIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<PyDocument> {
        self.documents.next()
    }
}
