//! Conversion of Python objects to YAML values, and of YAML nodes to plain
//! Python objects.

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PySet, PyString, PyTuple};
use rowan::ast::AstNode;
use yaml_edit::anchor_resolution::{AnchorRegistry, MergedMapping};
use yaml_edit::{Mapping, ScalarType, ScalarValue, Sequence, TaggedNode, YamlNode, YamlValue};

use crate::collections::{PyMapping, PySequence};
use crate::errors::{resolve_error, ResolveError, YamlError};
use crate::nodes::{PyAlias, PyNode, PyScalar, PyScalarValue, PyTaggedNode};

/// How deeply nested a Python value may be when converted to YAML, which also
/// stops a container that contains itself.
const MAX_DEPTH: usize = 512;

/// A Python value converted for a crate method that is generic over `AsYaml`.
///
/// Every scalar goes through `ScalarValue`, which is what the crate's own
/// `AsYaml` implementations for `&str`, integers, floats and `bool` build.
pub(crate) enum ValueArg {
    Scalar(ScalarValue),
    Node(YamlNode),
}

impl ValueArg {
    /// Convert a Python object.
    ///
    /// `None`, `bool`, `int`, `float` and `str` become scalars, a `dict` a
    /// block mapping and a `list` or `tuple` a block sequence. Node objects
    /// are inserted as a copy, and a `ScalarValue` keeps its style and tag.
    pub(crate) fn extract(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        Self::extract_nested(obj, 0)
    }

    fn extract_nested(obj: &Bound<'_, PyAny>, depth: usize) -> PyResult<Self> {
        if depth > MAX_DEPTH {
            return Err(PyValueError::new_err(
                "value is nested too deeply, or contains itself",
            ));
        }
        if obj.is_none() {
            return Ok(Self::Scalar(ScalarValue::null()));
        }
        // `bool` is checked before `int` because it is a subclass of `int`.
        if obj.is_instance_of::<PyBool>() {
            return Ok(Self::Scalar(ScalarValue::from(obj.extract::<bool>()?)));
        }
        if obj.is_instance_of::<PyInt>() {
            return Ok(Self::Scalar(match obj.extract::<i64>() {
                Ok(i) => ScalarValue::from(i),
                // Too large for i64; written out in full it is still an integer.
                Err(_) => ScalarValue::parse(obj.str()?.to_string()),
            }));
        }
        if obj.is_instance_of::<PyFloat>() {
            return Ok(Self::Scalar(ScalarValue::from(obj.extract::<f64>()?)));
        }
        if obj.is_instance_of::<PyString>() {
            return Ok(Self::Scalar(ScalarValue::string(obj.extract::<String>()?)));
        }
        if let Ok(value) = obj.extract::<PyRef<'_, PyScalarValue>>() {
            return Ok(Self::Scalar(value.inner.clone()));
        }
        if let Some(node) = node_of(obj) {
            return Ok(Self::Node(node));
        }
        if let Ok(dict) = obj.cast::<PyDict>() {
            let mapping = Mapping::new();
            for (key, value) in dict.iter() {
                let key = Self::extract_nested(&key, depth + 1)?;
                let value = Self::extract_nested(&value, depth + 1)?;
                call_with_key_value!(&key, &value, |k, v| mapping.set(k, v));
            }
            return Ok(Self::Node(YamlNode::Mapping(mapping)));
        }
        if obj.is_instance_of::<PyList>() || obj.is_instance_of::<PyTuple>() {
            let sequence = Sequence::new();
            for item in obj.try_iter()? {
                let item = Self::extract_nested(&item?, depth + 1)?;
                call_with_value!(&item, |v| sequence.push(v));
            }
            return Ok(Self::Node(YamlNode::Sequence(sequence)));
        }
        Err(PyTypeError::new_err(format!(
            "cannot convert {} to YAML",
            obj.get_type().name()?,
        )))
    }

    /// Convert `obj` for a comparison, or `None` if it has no YAML equivalent.
    pub(crate) fn extract_for_comparison(obj: &Bound<'_, PyAny>) -> PyResult<Option<Self>> {
        match Self::extract(obj) {
            Ok(value) => Ok(Some(value)),
            Err(e) if e.is_instance_of::<PyTypeError>(obj.py()) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// The YAML node a Python node object wraps, or `None` if `obj` is not one.
pub(crate) fn node_of(obj: &Bound<'_, PyAny>) -> Option<YamlNode> {
    if let Ok(node) = obj.extract::<PyRef<'_, PyNode>>() {
        return Some(node.inner.clone());
    }
    if let Ok(mapping) = obj.extract::<PyRef<'_, PyMapping>>() {
        return Some(YamlNode::Mapping(mapping.inner.clone()));
    }
    if let Ok(sequence) = obj.extract::<PyRef<'_, PySequence>>() {
        return Some(YamlNode::Sequence(sequence.inner.clone()));
    }
    if let Ok(scalar) = obj.extract::<PyRef<'_, PyScalar>>() {
        return Some(YamlNode::Scalar(scalar.inner.clone()));
    }
    if let Ok(alias) = obj.extract::<PyRef<'_, PyAlias>>() {
        return Some(YamlNode::Alias(alias.inner.clone()));
    }
    if let Ok(tagged) = obj.extract::<PyRef<'_, PyTaggedNode>>() {
        return Some(YamlNode::TaggedNode(tagged.inner.clone()));
    }
    None
}

/// Apply a method that is generic over `AsYaml` to a `&ValueArg`, as the
/// concrete type it holds.
///
/// Trait objects (`&dyn AsYaml`) do not themselves implement `AsYaml`, so the
/// call has to be monomorphized per variant.
macro_rules! call_with_value {
    ($arg:expr, |$v:ident| $body:expr) => {
        match $arg {
            $crate::convert::ValueArg::Scalar(s) => {
                let $v = s;
                $body
            }
            $crate::convert::ValueArg::Node(n) => {
                let $v = n;
                $body
            }
        }
    };
}
pub(crate) use call_with_value;

/// Like `call_with_value!`, for a method taking a key and a value.
macro_rules! call_with_key_value {
    ($key:expr, $value:expr, |$k:ident, $v:ident| $body:expr) => {
        $crate::convert::call_with_value!($key, |$k| $crate::convert::call_with_value!(
            $value,
            |$v| $body
        ))
    };
}
pub(crate) use call_with_key_value;

/// The syntax node of a YAML node.
pub(crate) fn syntax_of(node: &YamlNode) -> &rowan::SyntaxNode<yaml_edit::Lang> {
    match node {
        YamlNode::Scalar(n) => n.syntax(),
        YamlNode::Mapping(n) => n.syntax(),
        YamlNode::Sequence(n) => n.syntax(),
        YamlNode::Alias(n) => n.syntax(),
        YamlNode::TaggedNode(n) => n.syntax(),
    }
}

/// The anchors of the tree `node` belongs to.
pub(crate) fn registry_for(node: &YamlNode) -> AnchorRegistry {
    let root = syntax_of(node)
        .ancestors()
        .last()
        .expect("ancestors() includes the node itself");
    AnchorRegistry::from_tree(&root)
}

/// Convert a YAML node to a plain Python object, resolving aliases and merge
/// keys against the anchors of the tree it belongs to.
pub(crate) fn to_python(py: Python<'_>, node: &YamlNode) -> PyResult<Py<PyAny>> {
    let registry = registry_for(node);
    Converter {
        py,
        registry: &registry,
        open: Vec::new(),
    }
    .convert(node)
}

/// Convert a YAML node to a plain Python object, using `registry` to resolve
/// aliases.
pub(crate) fn to_python_with(
    py: Python<'_>,
    node: &YamlNode,
    registry: &AnchorRegistry,
) -> PyResult<Py<PyAny>> {
    Converter {
        py,
        registry,
        open: Vec::new(),
    }
    .convert(node)
}

struct Converter<'a, 'py> {
    py: Python<'py>,
    registry: &'a AnchorRegistry,
    /// The collections and aliases being converted, to detect recursion.
    open: Vec<rowan::TextRange>,
}

impl Converter<'_, '_> {
    fn convert(&mut self, node: &YamlNode) -> PyResult<Py<PyAny>> {
        let range = syntax_of(node).text_range();
        if self.open.contains(&range) {
            return Err(ResolveError::new_err(format!(
                "recursive alias or anchor at {range:?}"
            )));
        }
        self.open.push(range);
        let result = self.convert_inner(node);
        self.open.pop();
        result
    }

    fn convert_inner(&mut self, node: &YamlNode) -> PyResult<Py<PyAny>> {
        let py = self.py;
        match node {
            YamlNode::Scalar(scalar) => scalar_to_python(py, &ScalarValue::from_scalar(scalar)),
            YamlNode::Mapping(mapping) => self.mapping(mapping),
            YamlNode::Sequence(sequence) => {
                let items = sequence
                    .values()
                    .map(|item| self.convert(&item))
                    .collect::<PyResult<Vec<_>>>()?;
                Ok(PyList::new(py, items)?.into_any().unbind())
            }
            YamlNode::Alias(alias) => {
                let name = alias.name();
                // TODO: `AnchorRegistry::resolve` returns the last definition
                // of an anchor; an alias should see the last one before it.
                // The crate does not expose that lookup yet.
                let target = self
                    .registry
                    .resolve(&name)
                    .ok_or_else(|| ResolveError::new_err(format!("undefined alias {name:?}")))?;
                let target = YamlNode::from_syntax(target.clone())
                    .or_else(|| target.children().find_map(YamlNode::from_syntax))
                    .ok_or_else(|| {
                        ResolveError::new_err(format!("anchor {name:?} has no value"))
                    })?;
                self.convert(&target)
            }
            YamlNode::TaggedNode(tagged) => self.tagged(tagged),
        }
    }

    fn mapping(&mut self, mapping: &Mapping) -> PyResult<Py<PyAny>> {
        let merged = MergedMapping::new(mapping.clone(), self.registry).map_err(resolve_error)?;
        let dict = PyDict::new(self.py);
        for (key, value) in merged.iter() {
            dict.set_item(self.convert(&key)?, self.convert(&value)?)?;
        }
        Ok(dict.into_any().unbind())
    }

    fn tagged(&mut self, tagged: &TaggedNode) -> PyResult<Py<PyAny>> {
        let py = self.py;
        if let Some(set) = tagged.as_set() {
            let members = set
                .members()
                .map(|member| self.convert(&member))
                .collect::<PyResult<Vec<_>>>()?;
            return Ok(PySet::new(py, members)?.into_any().unbind());
        }
        if let Some(entries) = tagged.as_ordered_mapping() {
            let dict = PyDict::new(py);
            for entry in entries {
                let (key, value) = self.entry(&entry)?;
                dict.set_item(key, value)?;
            }
            return Ok(dict.into_any().unbind());
        }
        if let Some(entries) = tagged.as_pairs() {
            let pairs = entries
                .iter()
                .map(|entry| self.entry(entry))
                .collect::<PyResult<Vec<_>>>()?;
            return Ok(PyList::new(py, pairs)?.into_any().unbind());
        }
        // Other tags on collections, such as `!!map` or a local `!thing`,
        // do not change how they convert.
        if let Some(mapping) = tagged.as_mapping() {
            return self.mapping(&mapping);
        }
        if let Some(sequence) = tagged.as_sequence() {
            return self.convert(&YamlNode::Sequence(sequence));
        }
        if tagged
            .tag()
            .as_deref()
            .is_some_and(|tag| is_core_tag(tag, "binary"))
        {
            // TODO: the crate reads `!!binary` as a string here; decode it
            // ourselves until `YamlValue::cast` types it.
            let text: String = tagged
                .as_string()
                .ok_or_else(|| YamlError::new_err("!!binary value is not a scalar"))?
                .split_whitespace()
                .collect();
            let kwargs = PyDict::new(py);
            kwargs.set_item("validate", true)?;
            return Ok(py
                .import("base64")?
                .call_method("b64decode", (text,), Some(&kwargs))
                .map_err(|e| YamlError::new_err(format!("invalid !!binary value: {e}")))?
                .unbind());
        }
        match YamlValue::cast(tagged.syntax().clone()).map_err(resolve_error)? {
            Some(YamlValue::Scalar(value)) => scalar_to_python(py, &value),
            _ => Err(YamlError::new_err(format!(
                "cannot convert tagged node {tagged} to a Python value"
            ))),
        }
    }

    fn entry(&mut self, entry: &yaml_edit::MappingEntry) -> PyResult<(Py<PyAny>, Py<PyAny>)> {
        let key = match entry.key_node() {
            Some(key) => self.convert(&key)?,
            None => self.py.None(),
        };
        let value = match entry.value_node() {
            Some(value) => self.convert(&value)?,
            None => self.py.None(),
        };
        Ok((key, value))
    }
}

/// Whether `tag` is the YAML core tag `name`, written as `!!name` or in full.
fn is_core_tag(tag: &str, name: &str) -> bool {
    let full = tag
        .strip_prefix("!<")
        .and_then(|t| t.strip_suffix('>'))
        .unwrap_or(tag);
    tag.strip_prefix("!!") == Some(name) || full.strip_prefix("tag:yaml.org,2002:") == Some(name)
}

/// Convert a scalar to `None`, `bool`, `int`, `float`, `bytes` or `str`
/// according to its resolved type.
pub(crate) fn scalar_to_python(py: Python<'_>, value: &ScalarValue) -> PyResult<Py<PyAny>> {
    let mismatch =
        |kind: &str| YamlError::new_err(format!("{:?} is not a valid {kind}", value.value()));
    Ok(match value.scalar_type() {
        ScalarType::Null => py.None(),
        ScalarType::Boolean => PyBool::new(py, value.to_bool().ok_or_else(|| mismatch("boolean"))?)
            .to_owned()
            .into_any()
            .unbind(),
        ScalarType::Integer => match value.to_i64() {
            Some(i) => i.into_pyobject(py)?.into_any().unbind(),
            // Python's int has no size limit.
            None => {
                let text = value.value().trim().replace('_', "");
                py.get_type::<PyInt>()
                    .call1((text, 0))
                    .map_err(|_| mismatch("integer"))?
                    .unbind()
            }
        },
        ScalarType::Float => PyFloat::new(py, value.to_f64().ok_or_else(|| mismatch("float"))?)
            .into_any()
            .unbind(),
        ScalarType::Binary => match value.as_binary() {
            Some(Ok(bytes)) => PyBytes::new(py, &bytes).into_any().unbind(),
            Some(Err(e)) => return Err(YamlError::new_err(e)),
            None => return Err(mismatch("binary value")),
        },
        ScalarType::String | ScalarType::Timestamp | ScalarType::Regex => {
            PyString::new(py, value.value()).into_any().unbind()
        }
    })
}
