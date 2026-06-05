//! Python bindings for the `yaml-edit` crate.
//!
//! The wrapper types in `yaml-edit` are lightweight views over a shared rowan
//! syntax tree. rowan uses `Rc` internally, so the wrappers are neither `Send`
//! nor `Sync`; every `#[pyclass]` here is declared `unsendable`, which makes
//! PyO3 enforce single-thread access at runtime.

use pyo3::exceptions::{PyIndexError, PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyFloat, PyInt, PyString};
use std::str::FromStr;

use yaml_edit::{Document, Mapping, Scalar, Sequence, YamlFile, YamlNode};

/// A value accepted where YAML expects a scalar, mirroring the `AsYaml`
/// implementations available in the crate (`str`, integers, floats, `bool`).
enum ScalarArg {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl ScalarArg {
    /// Extract a scalar argument from a Python object.
    ///
    /// `bool` is checked before `int` because Python's `bool` is a subclass of
    /// `int`. `None` is rejected: the high-level crate API has no way to write a
    /// YAML null, so we surface that rather than writing a quoted "null" string.
    fn extract(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        if obj.is_none() {
            return Err(PyTypeError::new_err(
                "None is not supported as a YAML value; the editor cannot write null scalars",
            ));
        }
        if obj.is_instance_of::<PyBool>() {
            return Ok(ScalarArg::Bool(obj.extract()?));
        }
        if obj.is_instance_of::<PyInt>() {
            return Ok(ScalarArg::Int(obj.extract()?));
        }
        if obj.is_instance_of::<PyFloat>() {
            return Ok(ScalarArg::Float(obj.extract()?));
        }
        if obj.is_instance_of::<PyString>() {
            return Ok(ScalarArg::Str(obj.extract()?));
        }
        Err(PyTypeError::new_err(format!(
            "expected str, int, float or bool, got {}",
            obj.get_type().name()?,
        )))
    }
}

/// Apply an `AsYaml`-consuming method with the scalar argument as the matching
/// concrete type.
///
/// `yaml-edit`'s value methods are generic over `impl AsYaml`, and trait objects
/// (`&dyn AsYaml`) do not themselves implement `AsYaml`, so the call must be
/// monomorphized per arm rather than dispatched dynamically.
macro_rules! call_with_scalar {
    ($arg:expr, |$v:ident| $body:expr) => {
        match $arg {
            ScalarArg::Str(s) => {
                let $v = s.as_str();
                $body
            }
            ScalarArg::Int(i) => {
                let $v = i;
                $body
            }
            ScalarArg::Float(x) => {
                let $v = x;
                $body
            }
            ScalarArg::Bool(b) => {
                let $v = b;
                $body
            }
        }
    };
}

/// Wrap a YAML error as a Python `ValueError`.
fn map_err(err: yaml_edit::YamlError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

/// A YAML node returned from a getter: scalar, mapping, sequence, or other.
///
/// This is a read view; mutate through the owning `Mapping`/`Sequence`/`Document`.
#[pyclass(name = "Node", unsendable, module = "yaml_edit._yaml_edit")]
struct PyNode {
    inner: YamlNode,
}

#[pymethods]
impl PyNode {
    /// The kind of node: "scalar", "mapping", "sequence", "alias" or "tagged".
    #[getter]
    fn kind(&self) -> &'static str {
        match self.inner {
            YamlNode::Scalar(_) => "scalar",
            YamlNode::Mapping(_) => "mapping",
            YamlNode::Sequence(_) => "sequence",
            YamlNode::Alias(_) => "alias",
            YamlNode::TaggedNode(_) => "tagged",
        }
    }

    fn is_scalar(&self) -> bool {
        self.inner.is_scalar()
    }

    fn is_mapping(&self) -> bool {
        self.inner.is_mapping()
    }

    fn is_sequence(&self) -> bool {
        self.inner.is_sequence()
    }

    /// View this node as a `Scalar`, or `None` if it is not a scalar.
    fn as_scalar(&self) -> Option<PyScalar> {
        self.inner
            .as_scalar()
            .map(|s| PyScalar { inner: s.clone() })
    }

    /// View this node as a `Mapping`, or `None` if it is not a mapping.
    fn as_mapping(&self) -> Option<PyMapping> {
        self.inner
            .as_mapping()
            .map(|m| PyMapping { inner: m.clone() })
    }

    /// View this node as a `Sequence`, or `None` if it is not a sequence.
    fn as_sequence(&self) -> Option<PySequence> {
        self.inner
            .as_sequence()
            .map(|s| PySequence { inner: s.clone() })
    }

    /// The node's value as an `int`, if it is an integer scalar.
    fn as_int(&self) -> Option<i64> {
        self.inner.to_i64()
    }

    /// The node's value as a `float`, if it is a numeric scalar.
    fn as_float(&self) -> Option<f64> {
        self.inner.to_f64()
    }

    /// The node's value as a `bool`, if it is a boolean scalar.
    fn as_bool(&self) -> Option<bool> {
        self.inner.to_bool()
    }

    /// The node's source text, including any surrounding formatting.
    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Node kind={} {:?}>", self.kind(), self.__str__())
    }
}

/// A scalar YAML value (string, number, boolean).
#[pyclass(name = "Scalar", unsendable, module = "yaml_edit._yaml_edit")]
struct PyScalar {
    inner: Scalar,
}

#[pymethods]
impl PyScalar {
    /// The raw scalar text, preserving any surrounding quotes.
    fn value(&self) -> String {
        self.inner.value()
    }

    /// The scalar's value as a string, with quotes stripped if present.
    fn as_string(&self) -> String {
        self.inner.as_string()
    }

    fn is_quoted(&self) -> bool {
        self.inner.is_quoted()
    }

    fn unquoted_value(&self) -> String {
        self.inner.unquoted_value()
    }

    fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    fn as_int(&self) -> Option<i64> {
        self.inner.as_i64()
    }

    fn as_float(&self) -> Option<f64> {
        self.inner.as_f64()
    }

    fn as_bool(&self) -> Option<bool> {
        self.inner.as_bool()
    }

    /// Replace the scalar's value in place, preserving surrounding formatting.
    fn set_value(&self, value: &str) {
        self.inner.set_value(value);
    }

    fn __str__(&self) -> String {
        self.inner.as_string()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Scalar {:?}>", self.inner.as_string())
    }
}

/// A YAML mapping (key/value block).
#[pyclass(name = "Mapping", unsendable, module = "yaml_edit._yaml_edit")]
struct PyMapping {
    inner: Mapping,
}

#[pymethods]
impl PyMapping {
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    fn is_flow_style(&self) -> bool {
        self.inner.is_flow_style()
    }

    fn contains_key(&self, key: &str) -> bool {
        self.inner.contains_key(key)
    }

    fn __contains__(&self, key: &str) -> bool {
        self.inner.contains_key(key)
    }

    /// Look up a key, returning its value node or `None` if absent.
    fn get(&self, key: &str) -> Option<PyNode> {
        self.inner.get(key).map(|inner| PyNode { inner })
    }

    fn __getitem__(&self, key: &str) -> PyResult<PyNode> {
        self.inner
            .get(key)
            .map(|inner| PyNode { inner })
            .ok_or_else(|| PyKeyError::new_err(key.to_string()))
    }

    /// Look up a key whose value is a nested mapping.
    fn get_mapping(&self, key: &str) -> Option<PyMapping> {
        self.inner.get_mapping(key).map(|inner| PyMapping { inner })
    }

    /// Look up a key whose value is a nested sequence.
    fn get_sequence(&self, key: &str) -> Option<PySequence> {
        self.inner
            .get_sequence(key)
            .map(|inner| PySequence { inner })
    }

    /// Set `key` to `value`, inserting a new entry or updating an existing one.
    fn set(&self, key: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let arg = ScalarArg::extract(value)?;
        call_with_scalar!(&arg, |v| self.inner.set(key, v));
        Ok(())
    }

    fn __setitem__(&self, key: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.set(key, value)
    }

    /// Remove `key`, returning `True` if it was present.
    fn remove(&self, key: &str) -> bool {
        self.inner.remove(key).is_some()
    }

    fn __delitem__(&self, key: &str) -> PyResult<()> {
        if self.inner.remove(key).is_some() {
            Ok(())
        } else {
            Err(PyKeyError::new_err(key.to_string()))
        }
    }

    /// Rename `old_key` to `new_key`, returning `True` on success.
    fn rename_key(&self, old_key: &str, new_key: &str) -> bool {
        self.inner.rename_key(old_key, new_key)
    }

    fn clear(&self) {
        self.inner.clear();
    }

    /// The mapping's keys, in document order.
    fn keys(&self) -> Vec<String> {
        self.inner.keys().map(|k| k.to_string()).collect()
    }

    /// The mapping's value nodes, in document order.
    fn values(&self) -> Vec<PyNode> {
        self.inner.values().map(|inner| PyNode { inner }).collect()
    }

    /// (key, value) pairs, in document order.
    fn items(&self) -> Vec<(String, PyNode)> {
        self.inner
            .iter()
            .map(|(k, v)| (k.to_string(), PyNode { inner: v }))
            .collect()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Mapping len={}>", self.inner.len())
    }
}

/// A YAML sequence (list).
#[pyclass(name = "Sequence", unsendable, module = "yaml_edit._yaml_edit")]
struct PySequence {
    inner: Sequence,
}

#[pymethods]
impl PySequence {
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    fn is_flow_style(&self) -> bool {
        self.inner.is_flow_style()
    }

    /// Element at `index`, or `None` if out of range.
    fn get(&self, index: usize) -> Option<PyNode> {
        self.inner.get(index).map(|inner| PyNode { inner })
    }

    fn __getitem__(&self, index: usize) -> PyResult<PyNode> {
        self.inner
            .get(index)
            .map(|inner| PyNode { inner })
            .ok_or_else(|| PyIndexError::new_err("sequence index out of range"))
    }

    fn first(&self) -> Option<PyNode> {
        self.inner.first().map(|inner| PyNode { inner })
    }

    fn last(&self) -> Option<PyNode> {
        self.inner.last().map(|inner| PyNode { inner })
    }

    /// Append `value` to the end of the sequence.
    fn push(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let arg = ScalarArg::extract(value)?;
        call_with_scalar!(&arg, |v| self.inner.push(v));
        Ok(())
    }

    /// Insert `value` before `index`.
    fn insert(&self, index: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let arg = ScalarArg::extract(value)?;
        call_with_scalar!(&arg, |v| self.inner.insert(index, v));
        Ok(())
    }

    /// Replace the element at `index`, returning `True` on success.
    fn set(&self, index: usize, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        let arg = ScalarArg::extract(value)?;
        Ok(call_with_scalar!(&arg, |v| self.inner.set(index, v)))
    }

    fn __setitem__(&self, index: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let arg = ScalarArg::extract(value)?;
        if call_with_scalar!(&arg, |v| self.inner.set(index, v)) {
            Ok(())
        } else {
            Err(PyIndexError::new_err("sequence index out of range"))
        }
    }

    /// Remove and return the element at `index`.
    fn remove(&self, index: usize) -> Option<PyNode> {
        self.inner.remove(index).map(|inner| PyNode { inner })
    }

    /// Remove and return the last element.
    fn pop(&self) -> Option<PyNode> {
        self.inner.pop().map(|inner| PyNode { inner })
    }

    fn clear(&self) {
        self.inner.clear();
    }

    /// The sequence's element nodes, in order.
    fn values(&self) -> Vec<PyNode> {
        self.inner.values().map(|inner| PyNode { inner }).collect()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Sequence len={}>", self.inner.len())
    }
}

/// A single YAML document.
#[pyclass(name = "Document", unsendable, module = "yaml_edit._yaml_edit")]
struct PyDocument {
    inner: Document,
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
    #[staticmethod]
    fn parse(text: &str) -> PyResult<Self> {
        Document::from_str(text)
            .map(|inner| PyDocument { inner })
            .map_err(map_err)
    }

    /// Parse a document from a file on disk.
    #[staticmethod]
    fn from_file(path: &str) -> PyResult<Self> {
        Document::from_file(path)
            .map(|inner| PyDocument { inner })
            .map_err(map_err)
    }

    /// Write the document to a file on disk.
    fn to_file(&self, path: &str) -> PyResult<()> {
        self.inner.to_file(path).map_err(map_err)
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
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

    fn contains_key(&self, key: &str) -> bool {
        self.inner.contains_key(key)
    }

    fn __contains__(&self, key: &str) -> bool {
        self.inner.contains_key(key)
    }

    /// Look up a top-level key, returning its value node or `None`.
    fn get(&self, key: &str) -> Option<PyNode> {
        self.inner.get(key).map(|inner| PyNode { inner })
    }

    fn __getitem__(&self, key: &str) -> PyResult<PyNode> {
        self.inner
            .get(key)
            .map(|inner| PyNode { inner })
            .ok_or_else(|| PyKeyError::new_err(key.to_string()))
    }

    /// The value of a top-level key as a string, or `None`.
    fn get_string(&self, key: &str) -> Option<String> {
        self.inner.get_string(key)
    }

    /// Set a top-level key, treating the document root as a mapping.
    fn set(&self, key: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let arg = ScalarArg::extract(value)?;
        call_with_scalar!(&arg, |v| self.inner.set(key, v));
        Ok(())
    }

    fn __setitem__(&self, key: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.set(key, value)
    }

    /// Remove a top-level key, returning `True` if it was present.
    fn remove(&self, key: &str) -> bool {
        self.inner.remove(key).is_some()
    }

    fn __delitem__(&self, key: &str) -> PyResult<()> {
        if self.inner.remove(key).is_some() {
            Ok(())
        } else {
            Err(PyKeyError::new_err(key.to_string()))
        }
    }

    fn rename_key(&self, old_key: &str, new_key: &str) -> bool {
        self.inner.rename_key(old_key, new_key)
    }

    /// The top-level keys, in document order.
    fn keys(&self) -> Vec<String> {
        self.inner.keys().map(|k| k.to_string()).collect()
    }

    /// Serialize the document back to a string, preserving formatting.
    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Document {:?}>", self.__str__())
    }
}

/// A YAML stream, which may hold multiple documents and directives.
#[pyclass(name = "YamlFile", unsendable, module = "yaml_edit._yaml_edit")]
struct PyYamlFile {
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
    #[staticmethod]
    fn parse(text: &str) -> PyResult<Self> {
        YamlFile::from_str(text)
            .map(|inner| PyYamlFile { inner })
            .map_err(map_err)
    }

    /// Parse a YAML stream from a file on disk.
    #[staticmethod]
    fn from_file(path: &str) -> PyResult<Self> {
        YamlFile::from_path(path)
            .map(|inner| PyYamlFile { inner })
            .map_err(map_err)
    }

    fn __len__(&self) -> usize {
        self.inner.documents().count()
    }

    fn is_empty(&self) -> bool {
        self.inner.documents().next().is_none()
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
    fn push_document(&self, document: &PyDocument) {
        self.inner.push_document(document.inner.clone());
    }

    /// The directive lines (e.g. "%YAML 1.2") in this stream.
    fn directives(&self) -> Vec<String> {
        self.inner.directives().map(|d| d.to_string()).collect()
    }

    /// Add a directive line such as "%YAML 1.2".
    fn add_directive(&self, directive_text: &str) {
        self.inner.add_directive(directive_text);
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

#[pymodule]
fn _yaml_edit(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyDocument>()?;
    m.add_class::<PyYamlFile>()?;
    m.add_class::<PyMapping>()?;
    m.add_class::<PySequence>()?;
    m.add_class::<PyScalar>()?;
    m.add_class::<PyNode>()?;
    Ok(())
}
