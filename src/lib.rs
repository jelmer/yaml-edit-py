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

use yaml_edit::anchor_resolution::{AnchorRegistry, DocumentMergedExt, MergedMapping};
use yaml_edit::path::{AliasPolicy, PathError, YamlPath};
use yaml_edit::{
    byte_offset_to_line_column, Alias, Comment, Document, Mapping, MappingEntry, Scalar, Sequence,
    TaggedNode, TextPosition, YamlFile, YamlNode,
};

/// A value accepted where YAML expects a value, mirroring the `AsYaml`
/// implementations available in the crate.
enum ValueArg {
    Null,
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Node(YamlNode),
}

impl ValueArg {
    /// Extract a value argument from a Python object.
    ///
    /// `bool` is checked before `int` because Python's `bool` is a subclass of
    /// `int`. `None` becomes a YAML null, and the node wrappers (`Mapping`,
    /// `Sequence`, `Scalar`, `Node`, `Alias`, `TaggedNode`) are inserted as a
    /// copy of the node.
    fn extract(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        if obj.is_none() {
            return Ok(ValueArg::Null);
        }
        if obj.is_instance_of::<PyBool>() {
            return Ok(ValueArg::Bool(obj.extract()?));
        }
        if obj.is_instance_of::<PyInt>() {
            return Ok(ValueArg::Int(obj.extract()?));
        }
        if obj.is_instance_of::<PyFloat>() {
            return Ok(ValueArg::Float(obj.extract()?));
        }
        if obj.is_instance_of::<PyString>() {
            return Ok(ValueArg::Str(obj.extract()?));
        }
        if let Ok(node) = obj.extract::<PyRef<'_, PyNode>>() {
            return Ok(ValueArg::Node(node.inner.clone()));
        }
        if let Ok(mapping) = obj.extract::<PyRef<'_, PyMapping>>() {
            return Ok(ValueArg::Node(YamlNode::Mapping(mapping.inner.clone())));
        }
        if let Ok(sequence) = obj.extract::<PyRef<'_, PySequence>>() {
            return Ok(ValueArg::Node(YamlNode::Sequence(sequence.inner.clone())));
        }
        if let Ok(scalar) = obj.extract::<PyRef<'_, PyScalar>>() {
            return Ok(ValueArg::Node(YamlNode::Scalar(scalar.inner.clone())));
        }
        if let Ok(alias) = obj.extract::<PyRef<'_, PyAlias>>() {
            return Ok(ValueArg::Node(YamlNode::Alias(alias.inner.clone())));
        }
        if let Ok(tagged) = obj.extract::<PyRef<'_, PyTaggedNode>>() {
            return Ok(ValueArg::Node(YamlNode::TaggedNode(tagged.inner.clone())));
        }
        Err(PyTypeError::new_err(format!(
            "expected str, int, float, bool, None or a YAML node, got {}",
            obj.get_type().name()?,
        )))
    }
}

/// Apply an `AsYaml`-consuming method with the value argument as the matching
/// concrete type.
///
/// `yaml-edit`'s value methods are generic over `impl AsYaml`, and trait objects
/// (`&dyn AsYaml`) do not themselves implement `AsYaml`, so the call must be
/// monomorphized per arm rather than dispatched dynamically.
macro_rules! call_with_value {
    ($arg:expr, |$v:ident| $body:expr) => {
        match $arg {
            ValueArg::Null => {
                let $v = None::<&str>;
                $body
            }
            ValueArg::Str(s) => {
                let $v = s.as_str();
                $body
            }
            ValueArg::Int(i) => {
                let $v = i;
                $body
            }
            ValueArg::Float(x) => {
                let $v = x;
                $body
            }
            ValueArg::Bool(b) => {
                let $v = b;
                $body
            }
            ValueArg::Node(n) => {
                let $v = n;
                $body
            }
        }
    };
}

/// Wrap a YAML or alias resolution error as a Python `ValueError`.
fn map_err(err: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(err.to_string())
}

/// Convert a byte range to a Python `(start, end)` tuple.
fn span(range: TextPosition) -> (u32, u32) {
    (range.start, range.end)
}

/// The 1-based `(line, column)` of a byte offset in `source_text`.
fn line_column(source_text: &str, offset: u32) -> (usize, usize) {
    let lc = byte_offset_to_line_column(source_text, offset as usize);
    (lc.line, lc.column)
}

/// Parse an alias policy name ("refuse", "follow" or "expand").
fn parse_alias_policy(policy: &str) -> PyResult<AliasPolicy> {
    match policy {
        "refuse" => Ok(AliasPolicy::Refuse),
        "follow" => Ok(AliasPolicy::Follow),
        "expand" => Ok(AliasPolicy::Expand),
        _ => Err(PyValueError::new_err(format!(
            "invalid alias policy {policy:?}; expected \"refuse\", \"follow\" or \"expand\""
        ))),
    }
}

/// Look up `path`, returning `None` if the target does not exist (or the
/// document is empty) and raising `ValueError` for any other failure.
fn get_path(target: &impl YamlPath, path: &str, alias_policy: &str) -> PyResult<Option<PyNode>> {
    match target.try_get_path_with(path, parse_alias_policy(alias_policy)?) {
        Ok(inner) => Ok(Some(PyNode { inner })),
        Err(PathError::NotFound { .. } | PathError::NoRoot) => Ok(None),
        Err(e) => Err(map_err(e)),
    }
}

/// Set `path` to `value`, creating intermediate mappings as needed.
fn set_path(
    target: &impl YamlPath,
    path: &str,
    value: &Bound<'_, PyAny>,
    alias_policy: &str,
) -> PyResult<()> {
    let policy = parse_alias_policy(alias_policy)?;
    let arg = ValueArg::extract(value)?;
    call_with_value!(&arg, |v| target.try_set_path_with(path, v, policy)).map_err(map_err)
}

/// Remove `path`, returning `False` if the target does not exist (or the
/// document is empty) and raising `ValueError` for any other failure.
fn remove_path(target: &impl YamlPath, path: &str, alias_policy: &str) -> PyResult<bool> {
    match target.try_remove_path_with(path, parse_alias_policy(alias_policy)?) {
        Ok(_) => Ok(true),
        Err(PathError::NotFound { .. } | PathError::NoRoot) => Ok(false),
        Err(e) => Err(map_err(e)),
    }
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

    fn is_alias(&self) -> bool {
        self.inner.is_alias()
    }

    fn is_tagged(&self) -> bool {
        self.inner.is_tagged()
    }

    /// View this node as an `Alias`, or `None` if it is not an alias.
    fn as_alias(&self) -> Option<PyAlias> {
        match &self.inner {
            YamlNode::Alias(inner) => Some(PyAlias {
                inner: inner.clone(),
            }),
            _ => None,
        }
    }

    /// View this node as a `TaggedNode`, or `None` if it is not tagged.
    fn as_tagged(&self) -> Option<PyTaggedNode> {
        match &self.inner {
            YamlNode::TaggedNode(inner) => Some(PyTaggedNode {
                inner: inner.clone(),
            }),
            _ => None,
        }
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

    /// The `(start, end)` byte offsets of this node in the source text.
    fn byte_range(&self) -> (u32, u32) {
        span(self.inner.byte_range())
    }

    /// The 1-based `(line, column)` where this node starts.
    ///
    /// `source_text` is the text the tree was parsed from, or `str()` of the
    /// enclosing document after edits.
    fn start_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().start)
    }

    /// The 1-based `(line, column)` where this node ends.
    fn end_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().end)
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Node kind={} {:?}>", self.kind(), self.__str__())
    }
}

/// An alias (`*name`) referring to an anchored node.
#[pyclass(name = "Alias", unsendable, module = "yaml_edit._yaml_edit")]
struct PyAlias {
    inner: Alias,
}

#[pymethods]
impl PyAlias {
    /// Create an alias to the anchor `name`, for use as a value.
    #[new]
    fn new(name: &str) -> Self {
        PyAlias {
            inner: Alias::new(name),
        }
    }

    /// The anchor name, without the leading `*`.
    fn name(&self) -> String {
        self.inner.name()
    }

    fn __str__(&self) -> String {
        self.inner.value()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Alias {:?}>", self.inner.value())
    }
}

/// A node with an explicit tag, such as `!custom value` or `!!set`.
#[pyclass(name = "TaggedNode", unsendable, module = "yaml_edit._yaml_edit")]
struct PyTaggedNode {
    inner: TaggedNode,
}

#[pymethods]
impl PyTaggedNode {
    /// The tag as written, such as "!custom" or "!!set".
    fn tag(&self) -> Option<String> {
        self.inner.tag()
    }

    /// The tagged value as a `Scalar`, or `None` if it is a collection.
    fn value(&self) -> Option<PyScalar> {
        self.inner.value().map(|inner| PyScalar { inner })
    }

    /// The tagged value as a string, or `None` if it is a collection.
    fn as_string(&self) -> Option<String> {
        self.inner.as_string()
    }

    /// The tagged value as a `Mapping`, or `None` if it is not a mapping.
    fn as_mapping(&self) -> Option<PyMapping> {
        self.inner.as_mapping().map(|inner| PyMapping { inner })
    }

    /// The tagged value as a `Sequence`, or `None` if it is not a sequence.
    fn as_sequence(&self) -> Option<PySequence> {
        self.inner.as_sequence().map(|inner| PySequence { inner })
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.TaggedNode {:?}>", self.inner.to_string())
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

    /// Whether this is a block scalar (`|` literal or `>` folded).
    fn is_block(&self) -> bool {
        self.inner.is_block()
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

    /// The `(start, end)` byte offsets of this scalar in the source text.
    fn byte_range(&self) -> (u32, u32) {
        span(self.inner.byte_range())
    }

    /// The 1-based `(line, column)` where this scalar starts.
    ///
    /// `source_text` is the text the tree was parsed from, or `str()` of the
    /// enclosing document after edits.
    fn start_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().start)
    }

    /// The 1-based `(line, column)` where this scalar ends.
    fn end_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().end)
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Scalar {:?}>", self.inner.as_string())
    }
}

/// Reject comment text that would span lines, which `Comment::set_text`
/// panics on.
fn check_single_line(text: &str) -> PyResult<()> {
    if text.contains(['\n', '\r']) {
        return Err(PyValueError::new_err(format!(
            "comment text must not contain a newline, got {text:?}"
        )));
    }
    Ok(())
}

/// A comment (`# ...`) in the source text.
#[pyclass(name = "Comment", unsendable, module = "yaml_edit._yaml_edit")]
struct PyComment {
    inner: Comment,
}

#[pymethods]
impl PyComment {
    /// The full comment text, including the leading `#`.
    fn text(&self) -> String {
        self.inner.text().to_string()
    }

    /// The comment text without the leading `#` and one following space.
    fn content(&self) -> String {
        self.inner.content().to_string()
    }

    /// Replace the full comment text, which must start with `#`.
    ///
    /// Raises `ValueError` if `text` does not start with `#` or spans more
    /// than one line. This handle keeps reporting the old text afterwards;
    /// fetch the comment again through `comments()` to see the new one.
    fn set_text(&self, text: &str) -> PyResult<()> {
        if !text.starts_with('#') {
            return Err(PyValueError::new_err(format!(
                "comment text must start with '#', got {text:?}"
            )));
        }
        check_single_line(text)?;
        self.inner.set_text(text);
        Ok(())
    }

    /// Replace the comment content, keeping the `# ` prefix.
    ///
    /// Raises `ValueError` if `content` spans more than one line. As with
    /// `set_text`, this handle keeps reporting the old text afterwards.
    fn set_content(&self, content: &str) -> PyResult<()> {
        check_single_line(content)?;
        self.inner.set_content(content);
        Ok(())
    }

    /// The `(start, end)` byte offsets of this comment in the source text.
    fn byte_range(&self) -> (u32, u32) {
        span(self.inner.byte_range())
    }

    /// The 1-based `(line, column)` where this comment starts.
    ///
    /// `source_text` is the text the tree was parsed from, or `str()` of the
    /// enclosing document after edits.
    fn start_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().start)
    }

    /// The 1-based `(line, column)` where this comment ends.
    fn end_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().end)
    }

    fn __str__(&self) -> String {
        self.inner.text().to_string()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Comment {:?}>", self.inner.text())
    }
}

/// A single key/value entry in a mapping.
///
/// Entries are the fine-grained handle for working with mappings that contain
/// repeated keys: each occurrence of a key is a distinct entry, so they can be
/// inspected and mutated individually rather than collapsing to the first match.
#[pyclass(name = "Entry", unsendable, module = "yaml_edit._yaml_edit")]
struct PyEntry {
    inner: MappingEntry,
    /// Whether the owning mapping is flow-style, needed when rewriting the value.
    flow_context: bool,
}

#[pymethods]
impl PyEntry {
    /// The entry's key node, or `None` for a malformed entry with no key.
    fn key(&self) -> Option<PyNode> {
        self.inner.key_node().map(|inner| PyNode { inner })
    }

    /// The entry's value node, or `None` for a malformed entry with no value.
    fn value(&self) -> Option<PyNode> {
        self.inner.value_node().map(|inner| PyNode { inner })
    }

    /// Return `True` if this entry's key matches `key` (ignoring quoting style).
    fn key_matches(&self, key: &str) -> bool {
        self.inner.key_matches(key)
    }

    /// Replace this entry's value in place, preserving the key and formatting.
    fn set_value(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let arg = ValueArg::extract(value)?;
        call_with_value!(&arg, |v| self.inner.set_value(v, self.flow_context));
        Ok(())
    }

    /// Remove this entry from its mapping.
    ///
    /// The entry handle is detached from the tree; calling other methods on it
    /// afterwards operates on a node that is no longer part of the document.
    fn remove(&self) {
        self.inner.clone().remove();
    }

    /// The `(start, end)` byte offsets of this entry in the source text.
    fn byte_range(&self) -> (u32, u32) {
        span(self.inner.byte_range())
    }

    /// The 1-based `(line, column)` where this entry starts.
    ///
    /// `source_text` is the text the tree was parsed from, or `str()` of the
    /// enclosing document after edits.
    fn start_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().start)
    }

    /// The 1-based `(line, column)` where this entry ends.
    fn end_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().end)
    }

    fn __repr__(&self) -> String {
        let key = self
            .inner
            .key_node()
            .map(|k| k.to_string())
            .unwrap_or_default();
        format!("<yaml_edit.Entry key={key:?}>")
    }
}

/// A YAML mapping (key/value block).
#[pyclass(name = "Mapping", unsendable, module = "yaml_edit._yaml_edit")]
struct PyMapping {
    inner: Mapping,
}

#[pymethods]
impl PyMapping {
    /// Create an empty, detached mapping, for use as a value.
    ///
    /// Pass `flow=True` for a flow-style mapping (`{}`).
    #[new]
    #[pyo3(signature = (flow=false))]
    fn new(flow: bool) -> Self {
        PyMapping {
            inner: if flow {
                Mapping::new_flow()
            } else {
                Mapping::new()
            },
        }
    }

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
        let arg = ValueArg::extract(value)?;
        call_with_value!(&arg, |v| self.inner.set(key, v));
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

    /// Iterate over the keys, like a Python `dict`.
    fn __iter__(&self) -> PyKeyIterator {
        PyKeyIterator {
            keys: self.keys().into_iter(),
        }
    }

    /// All entries, in document order, as fine-grained handles.
    ///
    /// Unlike `items`, this exposes every occurrence of a repeated key as its
    /// own entry, and lets each be mutated in place.
    fn entries(&self) -> Vec<PyEntry> {
        let flow_context = self.inner.is_flow_style();
        self.inner
            .entries()
            .map(|inner| PyEntry {
                inner,
                flow_context,
            })
            .collect()
    }

    /// The first entry whose key matches `key`, or `None` if absent.
    fn find_entry(&self, key: &str) -> Option<PyEntry> {
        let flow_context = self.inner.is_flow_style();
        self.inner.find_entry_by_key(key).map(|inner| PyEntry {
            inner,
            flow_context,
        })
    }

    /// Every entry whose key matches `key`, in document order.
    ///
    /// YAML permits repeated keys; this returns one handle per occurrence.
    fn find_all_entries(&self, key: &str) -> Vec<PyEntry> {
        let flow_context = self.inner.is_flow_style();
        self.inner
            .find_all_entries_by_key(key)
            .map(|inner| PyEntry {
                inner,
                flow_context,
            })
            .collect()
    }

    /// The number of entries whose key matches `key`.
    fn count_key(&self, key: &str) -> usize {
        self.inner.find_all_entries_by_key(key).count()
    }

    /// Remove the `n`-th (0-based) occurrence of `key`, returning `True` if it
    /// existed.
    fn remove_nth(&self, key: &str, n: usize) -> bool {
        self.inner.remove_nth_occurrence(key, n).is_some()
    }

    /// Insert `key` immediately after `after_key`.
    ///
    /// An existing `key` is updated in place rather than moved. Returns
    /// `False` if `after_key` is not present.
    fn insert_after(&self, after_key: &str, key: &str, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        let arg = ValueArg::extract(value)?;
        Ok(call_with_value!(&arg, |v| self
            .inner
            .insert_after(after_key, key, v)))
    }

    /// Insert `key` immediately before `before_key`.
    ///
    /// An existing `key` is updated in place rather than moved. Returns
    /// `False` if `before_key` is not present.
    fn insert_before(
        &self,
        before_key: &str,
        key: &str,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let arg = ValueArg::extract(value)?;
        Ok(call_with_value!(&arg, |v| self
            .inner
            .insert_before(before_key, key, v)))
    }

    /// Set `key` and place it immediately after `after_key`, moving it if it
    /// already exists. Returns `False` if `after_key` is not present.
    fn move_after(&self, after_key: &str, key: &str, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        let arg = ValueArg::extract(value)?;
        Ok(call_with_value!(&arg, |v| self
            .inner
            .move_after(after_key, key, v)))
    }

    /// Set `key` and place it immediately before `before_key`, moving it if
    /// it already exists. Returns `False` if `before_key` is not present.
    fn move_before(&self, before_key: &str, key: &str, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        let arg = ValueArg::extract(value)?;
        Ok(call_with_value!(&arg, |v| self
            .inner
            .move_before(before_key, key, v)))
    }

    /// Insert `key` at position `index`, or at the end if `index` is past it.
    ///
    /// An existing `key` is updated in place rather than moved.
    fn insert_at_index(&self, index: usize, key: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let arg = ValueArg::extract(value)?;
        call_with_value!(&arg, |v| self.inner.insert_at_index(index, key, v));
        Ok(())
    }

    /// Reorder entries so the keys in `order` come first, in that order.
    ///
    /// Keys not in `order` follow, in their original relative order.
    fn reorder_fields(&self, order: Vec<String>) -> PyResult<()> {
        self.inner.reorder_fields(order);
        Ok(())
    }

    /// Look up a dotted path such as "server.ports[0]", or `None` if absent.
    ///
    /// Raises `ValueError` for a malformed path or one that descends into a
    /// scalar. `alias_policy` is "follow" (the default), "refuse" or "expand".
    #[pyo3(signature = (path, alias_policy="follow"))]
    fn get_path(&self, path: &str, alias_policy: &str) -> PyResult<Option<PyNode>> {
        get_path(&self.inner, path, alias_policy)
    }

    /// Set a dotted path, creating intermediate mappings as needed.
    ///
    /// Raises `ValueError` if the path cannot be set. Writing through an alias
    /// is refused by default; pass `alias_policy="follow"` to modify the
    /// anchored node, or "expand" to replace the alias with a copy first.
    #[pyo3(signature = (path, value, alias_policy="refuse"))]
    fn set_path(&self, path: &str, value: &Bound<'_, PyAny>, alias_policy: &str) -> PyResult<()> {
        set_path(&self.inner, path, value, alias_policy)
    }

    /// Remove a dotted path, returning `True` if it was present.
    ///
    /// Raises `ValueError` for a malformed path or one that descends into a
    /// scalar.
    #[pyo3(signature = (path, alias_policy="refuse"))]
    fn remove_path(&self, path: &str, alias_policy: &str) -> PyResult<bool> {
        remove_path(&self.inner, path, alias_policy)
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    /// The `(start, end)` byte offsets of this mapping in the source text.
    fn byte_range(&self) -> (u32, u32) {
        span(self.inner.byte_range())
    }

    /// The 1-based `(line, column)` where this mapping starts.
    ///
    /// `source_text` is the text the tree was parsed from, or `str()` of the
    /// enclosing document after edits.
    fn start_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().start)
    }

    /// The 1-based `(line, column)` where this mapping ends.
    fn end_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().end)
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Mapping len={}>", self.inner.len())
    }
}

/// Iterator over a mapping's keys, returned by `Mapping.__iter__`.
#[pyclass(module = "yaml_edit._yaml_edit")]
struct PyKeyIterator {
    keys: std::vec::IntoIter<String>,
}

#[pymethods]
impl PyKeyIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<String> {
        self.keys.next()
    }
}

/// A read-only view of a mapping with aliases and merge keys (`<<`) resolved.
///
/// Obtained from `Document.merged()`. Direct entries take precedence over keys
/// contributed by merge sources, the synthetic `<<` key is hidden, and aliases
/// (`*name`) are expanded to their anchored values.
///
/// The view does not mutate the document; mutate through the original
/// `Mapping`/`Document` instead.
///
/// Unlike the other collection types, this has no `__str__`: a merged view is
/// synthetic and has no single source span, so serializing it would have to
/// emit the un-merged source and hide the merged-in keys. Read the resolved
/// values through `items`/`get` instead.
#[pyclass(name = "MergedMapping", unsendable, module = "yaml_edit._yaml_edit")]
struct PyMergedMapping {
    base: Mapping,
    registry: AnchorRegistry,
}

impl PyMergedMapping {
    /// Borrow `base` and `registry` as a `MergedMapping` for a single query.
    ///
    /// The view is rebuilt on every query so that it reflects later edits to
    /// the document, which can also make it fail to resolve.
    fn view(&self) -> PyResult<MergedMapping<'_>> {
        MergedMapping::new(self.base.clone(), &self.registry).map_err(map_err)
    }
}

#[pymethods]
impl PyMergedMapping {
    fn __len__(&self) -> PyResult<usize> {
        Ok(self.view()?.len())
    }

    fn is_empty(&self) -> PyResult<bool> {
        Ok(self.view()?.is_empty())
    }

    fn contains_key(&self, key: &str) -> PyResult<bool> {
        Ok(self.view()?.contains_key(key))
    }

    fn __contains__(&self, key: &str) -> PyResult<bool> {
        Ok(self.view()?.contains_key(key))
    }

    /// Look up a key, resolving aliases and merge keys, or `None` if absent.
    fn get(&self, key: &str) -> PyResult<Option<PyNode>> {
        Ok(self.view()?.get(key).map(|inner| PyNode { inner }))
    }

    fn __getitem__(&self, key: &str) -> PyResult<PyNode> {
        self.view()?
            .get(key)
            .map(|inner| PyNode { inner })
            .ok_or_else(|| PyKeyError::new_err(key.to_string()))
    }

    /// Look up a nested mapping, returning another merged view that resolves
    /// against the same anchors.
    fn get_merged(&self, key: &str) -> PyResult<Option<PyMergedMapping>> {
        let nested = self.view()?.get_merged(key).map_err(map_err)?;
        Ok(nested.map(|nested| PyMergedMapping {
            base: nested.base().clone(),
            registry: self.registry.clone(),
        }))
    }

    /// The merged keys, in resolution order (direct entries first).
    fn keys(&self) -> PyResult<Vec<String>> {
        Ok(self.view()?.keys().map(|k| k.to_string()).collect())
    }

    /// The merged value nodes, in resolution order.
    fn values(&self) -> PyResult<Vec<PyNode>> {
        Ok(self
            .view()?
            .values()
            .map(|inner| PyNode { inner })
            .collect())
    }

    /// (key, value) pairs in the merged view, in resolution order.
    fn items(&self) -> PyResult<Vec<(String, PyNode)>> {
        Ok(self
            .view()?
            .iter()
            .map(|(k, v)| (k.to_string(), PyNode { inner: v }))
            .collect())
    }

    /// Iterate over the merged keys, like a Python `dict`.
    fn __iter__(&self) -> PyResult<PyKeyIterator> {
        Ok(PyKeyIterator {
            keys: self.keys()?.into_iter(),
        })
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "<yaml_edit.MergedMapping len={}>",
            self.view()?.len()
        ))
    }
}

/// A YAML sequence (list).
#[pyclass(name = "Sequence", unsendable, module = "yaml_edit._yaml_edit")]
struct PySequence {
    inner: Sequence,
}

#[pymethods]
impl PySequence {
    /// Create an empty, detached sequence, for use as a value.
    ///
    /// Pass `flow=True` for a flow-style sequence (`[]`).
    #[new]
    #[pyo3(signature = (flow=false))]
    fn new(flow: bool) -> Self {
        PySequence {
            inner: if flow {
                Sequence::new_flow()
            } else {
                Sequence::new()
            },
        }
    }

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
        let arg = ValueArg::extract(value)?;
        call_with_value!(&arg, |v| self.inner.push(v));
        Ok(())
    }

    /// Insert `value` before `index`.
    fn insert(&self, index: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let arg = ValueArg::extract(value)?;
        call_with_value!(&arg, |v| self.inner.insert(index, v));
        Ok(())
    }

    /// Replace the element at `index`, returning `True` on success.
    fn set(&self, index: usize, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        let arg = ValueArg::extract(value)?;
        Ok(call_with_value!(&arg, |v| self.inner.set(index, v)))
    }

    fn __setitem__(&self, index: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let arg = ValueArg::extract(value)?;
        if call_with_value!(&arg, |v| self.inner.set(index, v)) {
            Ok(())
        } else {
            Err(PyIndexError::new_err("sequence index out of range"))
        }
    }

    fn __delitem__(&self, index: usize) -> PyResult<()> {
        if self.inner.remove(index).is_some() {
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

    fn __iter__(&self) -> PyNodeIterator {
        PyNodeIterator {
            nodes: self.values().into_iter(),
        }
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    /// The `(start, end)` byte offsets of this sequence in the source text.
    fn byte_range(&self) -> (u32, u32) {
        span(self.inner.byte_range())
    }

    /// The 1-based `(line, column)` where this sequence starts.
    ///
    /// `source_text` is the text the tree was parsed from, or `str()` of the
    /// enclosing document after edits.
    fn start_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().start)
    }

    /// The 1-based `(line, column)` where this sequence ends.
    fn end_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().end)
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Sequence len={}>", self.inner.len())
    }
}

/// Iterator over a sequence's element nodes, returned by `Sequence.__iter__`.
#[pyclass(unsendable, module = "yaml_edit._yaml_edit")]
struct PyNodeIterator {
    nodes: std::vec::IntoIter<PyNode>,
}

#[pymethods]
impl PyNodeIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<PyNode> {
        self.nodes.next()
    }
}

/// A single YAML document.
#[pyclass(name = "Document", unsendable, module = "yaml_edit._yaml_edit")]
struct PyDocument {
    inner: Document,
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

    /// Parse a document, recovering from syntax errors instead of raising.
    ///
    /// Returns the document and a list of error messages, empty if the text
    /// parsed cleanly.
    #[staticmethod]
    fn parse_relaxed(text: &str) -> (Self, Vec<String>) {
        let (inner, errors) = Document::from_str_relaxed(text);
        (PyDocument { inner }, errors)
    }

    /// The comments in this document, in source order.
    fn comments(&self) -> Vec<PyComment> {
        self.inner
            .comments()
            .map(|inner| PyComment { inner })
            .collect()
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

    /// The document root as a tagged node, or `None` if it is not tagged.
    fn as_tagged(&self) -> Option<PyTaggedNode> {
        self.inner.as_tagged().map(|inner| PyTaggedNode { inner })
    }

    /// A merged view of the root mapping, resolving aliases and merge keys
    /// (`<<`), or `None` if the root is not a mapping.
    ///
    /// The anchor registry is built from the whole document, so aliases
    /// pointing at anchors defined elsewhere resolve correctly. Raises
    /// `ValueError` if an alias or merge key cannot be resolved.
    fn merged(&self) -> PyResult<Option<PyMergedMapping>> {
        let view = self.inner.merged().map_err(map_err)?;
        Ok(view.map(|view| PyMergedMapping {
            base: view.as_mapping().base().clone(),
            registry: view.registry().clone(),
        }))
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

    /// Set a top-level key, creating a root mapping if the document is empty.
    ///
    /// Raises `TypeError` if the root is not a mapping.
    fn set(&self, key: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let arg = ValueArg::extract(value)?;
        if call_with_value!(&arg, |v| self.inner.set(key, v)) {
            Ok(())
        } else {
            Err(PyTypeError::new_err("document root is not a mapping"))
        }
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

    /// Insert `key` immediately after `after_key`.
    ///
    /// An existing `key` is updated in place rather than moved. Returns
    /// `False` if `after_key` is not present.
    ///
    /// Raises `TypeError` if the root is not a mapping.
    fn insert_after(&self, after_key: &str, key: &str, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        let mapping = self.root_mapping()?;
        let arg = ValueArg::extract(value)?;
        Ok(call_with_value!(&arg, |v| mapping.insert_after(after_key, key, v)))
    }

    /// Insert `key` immediately before `before_key`.
    ///
    /// An existing `key` is updated in place rather than moved. Returns
    /// `False` if `before_key` is not present.
    ///
    /// Raises `TypeError` if the root is not a mapping.
    fn insert_before(
        &self,
        before_key: &str,
        key: &str,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let mapping = self.root_mapping()?;
        let arg = ValueArg::extract(value)?;
        Ok(call_with_value!(&arg, |v| mapping.insert_before(before_key, key, v)))
    }

    /// Set `key` and place it immediately after `after_key`, moving it if it
    /// already exists. Returns `False` if `after_key` is not present.
    ///
    /// Raises `TypeError` if the root is not a mapping.
    fn move_after(&self, after_key: &str, key: &str, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        let mapping = self.root_mapping()?;
        let arg = ValueArg::extract(value)?;
        Ok(call_with_value!(&arg, |v| mapping.move_after(after_key, key, v)))
    }

    /// Set `key` and place it immediately before `before_key`, moving it if
    /// it already exists. Returns `False` if `before_key` is not present.
    ///
    /// Raises `TypeError` if the root is not a mapping.
    fn move_before(&self, before_key: &str, key: &str, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        let mapping = self.root_mapping()?;
        let arg = ValueArg::extract(value)?;
        Ok(call_with_value!(&arg, |v| mapping.move_before(before_key, key, v)))
    }

    /// Insert `key` at position `index`, or at the end if `index` is past it.
    ///
    /// An existing `key` is updated in place rather than moved.
    ///
    /// Raises `TypeError` if the root is not a mapping.
    fn insert_at_index(&self, index: usize, key: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let arg = ValueArg::extract(value)?;
        if call_with_value!(&arg, |v| self.inner.insert_at_index(index, key, v)) {
            Ok(())
        } else {
            Err(PyTypeError::new_err("document root is not a mapping"))
        }
    }

    /// Reorder entries so the keys in `order` come first, in that order.
    ///
    /// Keys not in `order` follow, in their original relative order.
    ///
    /// Raises `TypeError` if the root is not a mapping.
    fn reorder_fields(&self, order: Vec<String>) -> PyResult<()> {
        if self.inner.reorder_fields(order) {
            Ok(())
        } else {
            Err(PyTypeError::new_err("document root is not a mapping"))
        }
    }

    /// Look up a dotted path such as "server.ports[0]", or `None` if absent.
    ///
    /// Raises `ValueError` for a malformed path or one that descends into a
    /// scalar. `alias_policy` is "follow" (the default), "refuse" or "expand".
    #[pyo3(signature = (path, alias_policy="follow"))]
    fn get_path(&self, path: &str, alias_policy: &str) -> PyResult<Option<PyNode>> {
        get_path(&self.inner, path, alias_policy)
    }

    /// Set a dotted path, creating intermediate mappings as needed.
    ///
    /// Raises `ValueError` if the path cannot be set. Writing through an alias
    /// is refused by default; pass `alias_policy="follow"` to modify the
    /// anchored node, or "expand" to replace the alias with a copy first.
    #[pyo3(signature = (path, value, alias_policy="refuse"))]
    fn set_path(&self, path: &str, value: &Bound<'_, PyAny>, alias_policy: &str) -> PyResult<()> {
        set_path(&self.inner, path, value, alias_policy)
    }

    /// Remove a dotted path, returning `True` if it was present.
    ///
    /// Raises `ValueError` for a malformed path or one that descends into a
    /// scalar.
    #[pyo3(signature = (path, alias_policy="refuse"))]
    fn remove_path(&self, path: &str, alias_policy: &str) -> PyResult<bool> {
        remove_path(&self.inner, path, alias_policy)
    }

    /// Serialize the document back to a string, preserving formatting.
    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    /// The `(start, end)` byte offsets of this document in the source text.
    fn byte_range(&self) -> (u32, u32) {
        span(self.inner.byte_range())
    }

    /// The 1-based `(line, column)` where this document starts.
    ///
    /// `source_text` is the text the tree was parsed from, or `str()` of the
    /// enclosing document after edits.
    fn start_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().start)
    }

    /// The 1-based `(line, column)` where this document ends.
    fn end_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().end)
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

    /// Parse a YAML stream, recovering from syntax errors instead of raising.
    ///
    /// Returns the stream and a list of error messages, empty if the text
    /// parsed cleanly.
    #[staticmethod]
    fn parse_relaxed(text: &str) -> (Self, Vec<String>) {
        let (inner, errors) = YamlFile::from_str_relaxed(text);
        (PyYamlFile { inner }, errors)
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

    /// The comments in this stream, in source order.
    fn comments(&self) -> Vec<PyComment> {
        self.inner
            .comments()
            .map(|inner| PyComment { inner })
            .collect()
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

/// The 1-based `(line, column)` of byte offset `offset` in `text`.
///
/// Columns count characters, not bytes.
#[pyfunction(name = "byte_offset_to_line_column")]
fn py_byte_offset_to_line_column(text: &str, offset: u32) -> (usize, usize) {
    line_column(text, offset)
}

#[pymodule]
fn _yaml_edit(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyDocument>()?;
    m.add_class::<PyYamlFile>()?;
    m.add_class::<PyMapping>()?;
    m.add_class::<PyMergedMapping>()?;
    m.add_class::<PySequence>()?;
    m.add_class::<PyScalar>()?;
    m.add_class::<PyNode>()?;
    m.add_class::<PyEntry>()?;
    m.add_class::<PyAlias>()?;
    m.add_class::<PyTaggedNode>()?;
    m.add_class::<PyComment>()?;
    m.add_function(wrap_pyfunction!(py_byte_offset_to_line_column, m)?)?;
    Ok(())
}
