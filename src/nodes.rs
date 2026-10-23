//! Wrappers for individual YAML nodes: `Node`, `Scalar`, `Alias`,
//! `TaggedNode`, `Comment`, `Entry`, and `ScalarValue` for writing scalars.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyString};
use yaml_edit::{
    Alias, Comment, MappingEntry, Scalar, ScalarStyle, ScalarType, ScalarValue, TaggedNode,
    YamlNode,
};

use crate::collections::{PyMapping, PySequence};
use crate::convert::{call_with_value, node_of, scalar_to_python, to_python, ValueArg};
use crate::{compare, line_column, python_hash, span};

/// A YAML node returned from a getter: scalar, mapping, sequence, alias or
/// tagged node.
///
/// This is a read view; mutate through the owning `Mapping`/`Sequence`/`Document`.
#[pyclass(name = "Node", unsendable, module = "yaml_edit._yaml_edit")]
pub(crate) struct PyNode {
    pub(crate) inner: YamlNode,
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

    /// The node's value as a `float`, if it is a float scalar.
    fn as_float(&self) -> Option<f64> {
        self.inner.to_f64()
    }

    /// The node's value as a `bool`, if it is a boolean scalar.
    fn as_bool(&self) -> Option<bool> {
        self.inner.to_bool()
    }

    /// Convert to plain Python objects (`dict`, `list`, `str`, `int`, ...),
    /// resolving aliases and merge keys.
    fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, &self.inner)
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

    /// Compare by YAML value, ignoring formatting: `doc["port"] == 8080`.
    ///
    /// Aliases compare equal to aliases with the same anchor name.
    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let YamlNode::Alias(alias) = &self.inner {
            return Ok(compare_alias(py, alias, other));
        }
        compare(py, &self.inner, other)
    }

    /// The hash of `to_python()`, so that equal values hash alike.
    fn __hash__(&self, py: Python<'_>) -> PyResult<isize> {
        python_hash(py, &self.inner)
    }

    /// The node's source text, including any surrounding formatting.
    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Node kind={} {:?}>", self.kind(), self.__str__())
    }
}

/// An alias (`*name`) referring to an anchored node.
#[pyclass(name = "Alias", unsendable, module = "yaml_edit._yaml_edit")]
pub(crate) struct PyAlias {
    pub(crate) inner: Alias,
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

    /// Compare with another alias by anchor name.
    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> Py<PyAny> {
        compare_alias(py, &self.inner, other)
    }

    fn __hash__(&self, py: Python<'_>) -> PyResult<isize> {
        PyString::new(py, &self.inner.name()).hash()
    }

    fn __str__(&self) -> String {
        self.inner.value()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Alias {:?}>", self.inner.value())
    }
}

/// Compare an alias with `other` by anchor name, or `NotImplemented` if
/// `other` is not an alias.
fn compare_alias(py: Python<'_>, alias: &Alias, other: &Bound<'_, PyAny>) -> Py<PyAny> {
    match node_of(other) {
        Some(YamlNode::Alias(other)) => PyBool::new(py, alias.name() == other.name())
            .to_owned()
            .into_any()
            .unbind(),
        _ => py.NotImplemented(),
    }
}

/// A node with an explicit tag, such as `!custom value` or `!!set`.
#[pyclass(name = "TaggedNode", unsendable, module = "yaml_edit._yaml_edit")]
pub(crate) struct PyTaggedNode {
    pub(crate) inner: TaggedNode,
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

    /// The members of a `!!set`, or `None` if this is not a set.
    fn as_set(&self) -> Option<Vec<PyNode>> {
        self.inner
            .as_set()
            .map(|set| set.members().map(|inner| PyNode { inner }).collect())
    }

    /// The `(key, value)` entries of a `!!omap`, or `None` if this is not one.
    fn as_ordered_mapping(&self) -> Option<Vec<(Option<PyNode>, Option<PyNode>)>> {
        self.inner.as_ordered_mapping().map(entry_pairs)
    }

    /// The `(key, value)` entries of a `!!pairs`, or `None` if this is not one.
    fn as_pairs(&self) -> Option<Vec<(Option<PyNode>, Option<PyNode>)>> {
        self.inner.as_pairs().map(entry_pairs)
    }

    /// Convert to plain Python objects: a `!!set` becomes a `set`, a `!!omap`
    /// a `dict` and `!!pairs` a list of tuples; other tags are dropped.
    fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, &YamlNode::TaggedNode(self.inner.clone()))
    }

    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        compare(py, &self.inner, other)
    }

    fn __hash__(&self, py: Python<'_>) -> PyResult<isize> {
        python_hash(py, &YamlNode::TaggedNode(self.inner.clone()))
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.TaggedNode {:?}>", self.inner.to_string())
    }
}

fn entry_pairs(entries: Vec<MappingEntry>) -> Vec<(Option<PyNode>, Option<PyNode>)> {
    entries
        .into_iter()
        .map(|entry| {
            (
                entry.key_node().map(|inner| PyNode { inner }),
                entry.value_node().map(|inner| PyNode { inner }),
            )
        })
        .collect()
}

fn style_name(style: ScalarStyle) -> &'static str {
    match style {
        ScalarStyle::Plain => "plain",
        ScalarStyle::SingleQuoted => "single-quoted",
        ScalarStyle::DoubleQuoted => "double-quoted",
        ScalarStyle::Literal => "literal",
        ScalarStyle::Folded => "folded",
    }
}

fn parse_style(style: &str) -> PyResult<ScalarStyle> {
    match style {
        "plain" => Ok(ScalarStyle::Plain),
        "single-quoted" => Ok(ScalarStyle::SingleQuoted),
        "double-quoted" => Ok(ScalarStyle::DoubleQuoted),
        "literal" => Ok(ScalarStyle::Literal),
        "folded" => Ok(ScalarStyle::Folded),
        _ => Err(PyValueError::new_err(format!(
            "invalid scalar style {style:?}; expected \"plain\", \"single-quoted\", \
             \"double-quoted\", \"literal\" or \"folded\""
        ))),
    }
}

fn type_name(scalar_type: ScalarType) -> &'static str {
    match scalar_type {
        ScalarType::String => "string",
        ScalarType::Integer => "integer",
        ScalarType::Float => "float",
        ScalarType::Boolean => "boolean",
        ScalarType::Null => "null",
        ScalarType::Binary => "binary",
        ScalarType::Timestamp => "timestamp",
        ScalarType::Regex => "regex",
    }
}

/// A scalar YAML value (string, number, boolean, null).
#[pyclass(name = "Scalar", unsendable, module = "yaml_edit._yaml_edit")]
pub(crate) struct PyScalar {
    pub(crate) inner: Scalar,
}

#[pymethods]
impl PyScalar {
    /// The raw scalar text, preserving any surrounding quotes.
    fn value(&self) -> String {
        self.inner.value()
    }

    /// The scalar's value as a string, with quotes and escapes resolved.
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

    /// Whether this is a block scalar (`|` literal or `>` folded).
    fn is_block(&self) -> bool {
        self.inner.is_block()
    }

    /// The scalar's style: "plain", "single-quoted", "double-quoted",
    /// "literal" or "folded".
    fn style(&self) -> &'static str {
        style_name(ScalarValue::from_scalar(&self.inner).style())
    }

    /// The scalar's resolved type: "string", "integer", "float", "boolean",
    /// "null", "binary", "timestamp" or "regex".
    fn scalar_type(&self) -> &'static str {
        type_name(ScalarValue::from_scalar(&self.inner).scalar_type())
    }

    /// Replace the scalar's raw text in place, preserving surrounding
    /// formatting. The text is written as given, without quoting.
    fn set_value(&self, value: &str) {
        self.inner.set_value(value);
    }

    /// Convert to `None`, `bool`, `int`, `float` or `str` according to the
    /// scalar's resolved type.
    fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        scalar_to_python(py, &ScalarValue::from_scalar(&self.inner))
    }

    /// The `(start, end)` byte offsets of this scalar in the source text.
    fn byte_range(&self) -> (u32, u32) {
        span(self.inner.byte_range())
    }

    /// The 1-based `(line, column)` where this scalar starts.
    fn start_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().start)
    }

    /// The 1-based `(line, column)` where this scalar ends.
    fn end_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().end)
    }

    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        compare(py, &self.inner, other)
    }

    fn __hash__(&self, py: Python<'_>) -> PyResult<isize> {
        python_hash(py, &YamlNode::Scalar(self.inner.clone()))
    }

    fn __str__(&self) -> String {
        self.inner.as_string()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Scalar {:?}>", self.inner.as_string())
    }
}

/// A scalar to write with an explicit style or tag.
///
/// Pass it wherever a value is accepted, for example
/// `doc["version"] = ScalarValue("1.0", style="double-quoted")`.
#[pyclass(name = "ScalarValue", module = "yaml_edit._yaml_edit")]
pub(crate) struct PyScalarValue {
    pub(crate) inner: ScalarValue,
}

#[pymethods]
impl PyScalarValue {
    /// Create a scalar from its text, in `style` ("plain", "single-quoted",
    /// "double-quoted", "literal" or "folded") and with an optional `tag`.
    ///
    /// The value is a string; quoting is added or escaped as the style needs.
    #[new]
    #[pyo3(signature = (value, style="plain", tag=None))]
    fn new(value: &str, style: &str, tag: Option<&str>) -> PyResult<Self> {
        let inner = ScalarValue::with_style(value, parse_style(style)?);
        Ok(PyScalarValue {
            inner: match tag {
                Some(tag) => inner.with_tag(tag),
                None => inner,
            },
        })
    }

    /// The scalar's text.
    fn value(&self) -> &str {
        self.inner.value()
    }

    fn style(&self) -> &'static str {
        style_name(self.inner.style())
    }

    fn tag(&self) -> Option<&str> {
        self.inner.tag()
    }

    /// The scalar rendered as YAML.
    fn __str__(&self) -> String {
        self.inner.to_yaml_string()
    }

    fn __repr__(&self) -> String {
        format!(
            "ScalarValue({:?}, style={:?}, tag={:?})",
            self.inner.value(),
            self.style(),
            self.inner.tag()
        )
    }
}

/// A comment (`# ...`) in the source text.
#[pyclass(name = "Comment", unsendable, module = "yaml_edit._yaml_edit")]
pub(crate) struct PyComment {
    pub(crate) inner: Comment,
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
    /// Raises `ValueError` if `text` does not start with `#`, spans more
    /// than one line or holds a character YAML does not allow.
    fn set_text(&mut self, text: &str) -> PyResult<()> {
        self.inner
            .set_text(text)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// Replace the comment content, keeping the `# ` prefix.
    ///
    /// Raises `ValueError` as `set_text` does.
    fn set_content(&mut self, content: &str) -> PyResult<()> {
        self.inner
            .set_content(content)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// The `(start, end)` byte offsets of this comment in the source text.
    fn byte_range(&self) -> (u32, u32) {
        span(self.inner.byte_range())
    }

    /// The 1-based `(line, column)` where this comment starts.
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
pub(crate) struct PyEntry {
    pub(crate) inner: MappingEntry,
    /// Whether the owning mapping is flow-style, needed when rewriting the value.
    pub(crate) flow_context: bool,
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

    /// Return `True` if this entry's key equals `key` (ignoring quoting style).
    fn key_matches(&self, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        let key = ValueArg::extract(key)?;
        Ok(call_with_value!(&key, |k| self.inner.key_matches(k)))
    }

    /// Replace this entry's value in place, preserving the key and formatting.
    fn set_value(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = ValueArg::extract(value)?;
        call_with_value!(&value, |v| self.inner.set_value(v, self.flow_context));
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
