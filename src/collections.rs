//! `Mapping`, `MergedMapping` and `Sequence`, with the `dict` and `list`
//! protocols, and the dict-like operations `Document` shares with `Mapping`.

use std::ffi::CStr;

use pyo3::exceptions::{PyDeprecationWarning, PyIndexError, PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyInt, PySlice, PySliceMethods, PyTuple};
use yaml_edit::anchor_resolution::{AnchorRegistry, MergedMapping};
use yaml_edit::{Document, Mapping, MappingEntry, ScalarValue, Sequence, YamlNode};

use crate::convert::{
    call_with_key_value, call_with_value, registry_for, to_python, to_python_with, ValueArg,
};
use crate::errors::resolve_error;
use crate::nodes::{PyEntry, PyNode};
use crate::{compare, line_column, span};

/// Emit a `DeprecationWarning` pointing at the caller.
pub(crate) fn deprecated(py: Python<'_>, message: &CStr) -> PyResult<()> {
    PyErr::warn(py, &py.get_type::<PyDeprecationWarning>(), message, 1)
}

/// Convert a field order to scalars, which `set_with_field_order` and
/// `reorder_fields` need all of one type.
pub(crate) fn field_order(order: &Bound<'_, PyAny>) -> PyResult<Vec<ScalarValue>> {
    order
        .try_iter()?
        .map(|key| match ValueArg::extract(&key?)? {
            ValueArg::Scalar(s) => Ok(s),
            ValueArg::Node(_) => Err(PyTypeError::new_err("field order keys must be scalars")),
        })
        .collect()
}

/// Something with `dict`-like access to a mapping: a `Mapping`, or a
/// `Document` through its root mapping.
pub(crate) trait MappingLike {
    /// The mapping to read from, or `None` if there is none.
    fn mapping(&self) -> Option<Mapping>;

    /// Set `key` to `value`.
    fn set(&self, key: &ValueArg, value: &ValueArg) -> PyResult<()>;

    /// Set `key` to `value`, placing a new key according to `order`.
    fn set_ordered(&self, key: &ValueArg, value: &ValueArg, order: &[ScalarValue]) -> PyResult<()>;
}

impl MappingLike for Mapping {
    fn mapping(&self) -> Option<Mapping> {
        Some(self.clone())
    }

    fn set(&self, key: &ValueArg, value: &ValueArg) -> PyResult<()> {
        call_with_key_value!(key, value, |k, v| Mapping::set(self, k, v));
        Ok(())
    }

    fn set_ordered(&self, key: &ValueArg, value: &ValueArg, order: &[ScalarValue]) -> PyResult<()> {
        call_with_key_value!(key, value, |k, v| self.set_with_field_order(k, v, order));
        Ok(())
    }
}

fn not_a_mapping() -> PyErr {
    PyTypeError::new_err("document root is not a mapping")
}

impl MappingLike for Document {
    fn mapping(&self) -> Option<Mapping> {
        self.as_mapping()
    }

    /// Set a key, creating a root mapping if the document is empty.
    fn set(&self, key: &ValueArg, value: &ValueArg) -> PyResult<()> {
        if call_with_key_value!(key, value, |k, v| Document::set(self, k, v)) {
            Ok(())
        } else {
            Err(not_a_mapping())
        }
    }

    fn set_ordered(&self, key: &ValueArg, value: &ValueArg, order: &[ScalarValue]) -> PyResult<()> {
        if call_with_key_value!(key, value, |k, v| self.set_with_field_order(k, v, order)) {
            Ok(())
        } else {
            Err(not_a_mapping())
        }
    }
}

pub(crate) fn node(py: Python<'_>, inner: YamlNode) -> PyResult<Py<PyAny>> {
    Ok(Bound::new(py, PyNode { inner })?.into_any().unbind())
}

pub(crate) fn get(target: &impl MappingLike, key: &Bound<'_, PyAny>) -> PyResult<Option<YamlNode>> {
    let Some(mapping) = target.mapping() else {
        return Ok(None);
    };
    let key = ValueArg::extract(key)?;
    Ok(call_with_value!(&key, |k| mapping.get(k)))
}

pub(crate) fn get_or(
    py: Python<'_>,
    target: &impl MappingLike,
    key: &Bound<'_, PyAny>,
    default: Option<Py<PyAny>>,
) -> PyResult<Py<PyAny>> {
    match get(target, key)? {
        Some(inner) => node(py, inner),
        None => Ok(default.unwrap_or_else(|| py.None())),
    }
}

pub(crate) fn getitem(target: &impl MappingLike, key: &Bound<'_, PyAny>) -> PyResult<PyNode> {
    get(target, key)?
        .map(|inner| PyNode { inner })
        .ok_or_else(|| PyKeyError::new_err(key.clone().unbind()))
}

pub(crate) fn contains(target: &impl MappingLike, key: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(get(target, key)?.is_some())
}

pub(crate) fn set(
    target: &impl MappingLike,
    key: &Bound<'_, PyAny>,
    value: &Bound<'_, PyAny>,
    field_order: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let key = ValueArg::extract(key)?;
    let value = ValueArg::extract(value)?;
    match field_order {
        Some(order) => target.set_ordered(&key, &value, &self::field_order(order)?),
        None => target.set(&key, &value),
    }
}

fn remove_entry(
    target: &impl MappingLike,
    key: &Bound<'_, PyAny>,
) -> PyResult<Option<MappingEntry>> {
    let Some(mapping) = target.mapping() else {
        return Ok(None);
    };
    let key = ValueArg::extract(key)?;
    Ok(call_with_value!(&key, |k| mapping.remove(k)))
}

pub(crate) fn remove(target: &impl MappingLike, key: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(remove_entry(target, key)?.is_some())
}

pub(crate) fn delitem(target: &impl MappingLike, key: &Bound<'_, PyAny>) -> PyResult<()> {
    match remove_entry(target, key)? {
        Some(_) => Ok(()),
        None => Err(PyKeyError::new_err(key.clone().unbind())),
    }
}

/// Remove `key` and return its value node; `default` holds the optional
/// fallback, taken as `*args` so that an explicit `None` counts.
pub(crate) fn pop(
    py: Python<'_>,
    target: &impl MappingLike,
    key: &Bound<'_, PyAny>,
    default: &Bound<'_, PyTuple>,
) -> PyResult<Py<PyAny>> {
    if default.len() > 1 {
        return Err(PyTypeError::new_err(format!(
            "pop expected at most 2 arguments, got {}",
            default.len() + 1
        )));
    }
    match remove_entry(target, key)? {
        Some(entry) => match entry.value_node() {
            Some(inner) => node(py, inner),
            None => Ok(py.None()),
        },
        None => match default.iter().next() {
            Some(default) => Ok(default.unbind()),
            None => Err(PyKeyError::new_err(key.clone().unbind())),
        },
    }
}

pub(crate) fn popitem(
    py: Python<'_>,
    target: &impl MappingLike,
) -> PyResult<(Py<PyAny>, Py<PyAny>)> {
    let entry = target
        .mapping()
        .and_then(|mapping| mapping.entries().last())
        .ok_or_else(|| PyKeyError::new_err("popitem(): mapping is empty"))?;
    let key = match entry.key_node() {
        Some(key) => to_python(py, &key)?,
        None => py.None(),
    };
    let value = match entry.value_node() {
        Some(inner) => node(py, inner)?,
        None => py.None(),
    };
    entry.remove();
    Ok((key, value))
}

pub(crate) fn setdefault(
    target: &impl MappingLike,
    key: &Bound<'_, PyAny>,
    default: &Bound<'_, PyAny>,
) -> PyResult<PyNode> {
    if let Some(inner) = get(target, key)? {
        return Ok(PyNode { inner });
    }
    set(target, key, default, None)?;
    getitem(target, key)
}

/// Set keys from a mapping (anything with `keys()`) or an iterable of pairs,
/// then from keyword arguments, like `dict.update`.
pub(crate) fn update(
    target: &impl MappingLike,
    other: Option<&Bound<'_, PyAny>>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<()> {
    if let Some(other) = other {
        if other.hasattr("keys")? {
            for key in other.call_method0("keys")?.try_iter()? {
                let key = key?;
                set(target, &key, &other.get_item(&key)?, None)?;
            }
        } else {
            for item in other.try_iter()? {
                let (key, value): (Bound<'_, PyAny>, Bound<'_, PyAny>) = item?.extract()?;
                set(target, &key, &value, None)?;
            }
        }
    }
    if let Some(kwargs) = kwargs {
        for (key, value) in kwargs.iter() {
            set(target, &key, &value, None)?;
        }
    }
    Ok(())
}

/// The mapping's keys as Python values, resolving any aliases.
pub(crate) fn keys(py: Python<'_>, target: &impl MappingLike) -> PyResult<Vec<Py<PyAny>>> {
    let Some(mapping) = target.mapping() else {
        return Ok(Vec::new());
    };
    let registry = registry_for(&YamlNode::Mapping(mapping.clone()));
    mapping
        .keys()
        .map(|key| to_python_with(py, &key, &registry))
        .collect()
}

pub(crate) fn values(target: &impl MappingLike) -> Vec<PyNode> {
    target
        .mapping()
        .map(|mapping| mapping.values().map(|inner| PyNode { inner }).collect())
        .unwrap_or_default()
}

pub(crate) fn items(
    py: Python<'_>,
    target: &impl MappingLike,
) -> PyResult<Vec<(Py<PyAny>, PyNode)>> {
    let Some(mapping) = target.mapping() else {
        return Ok(Vec::new());
    };
    let registry = registry_for(&YamlNode::Mapping(mapping.clone()));
    mapping
        .iter()
        .map(|(key, inner)| Ok((to_python_with(py, &key, &registry)?, PyNode { inner })))
        .collect()
}

pub(crate) fn rename_key(
    target: &impl MappingLike,
    old_key: &Bound<'_, PyAny>,
    new_key: &Bound<'_, PyAny>,
) -> PyResult<bool> {
    let Some(mapping) = target.mapping() else {
        return Ok(false);
    };
    let old_key = ValueArg::extract(old_key)?;
    let new_key = ValueArg::extract(new_key)?;
    Ok(call_with_key_value!(&old_key, &new_key, |o, n| mapping.rename_key(o, n)))
}

/// Iterator over Python values, returned by the `__iter__` of mappings.
#[pyclass(module = "yaml_edit._yaml_edit")]
pub(crate) struct PyKeyIterator {
    pub(crate) keys: std::vec::IntoIter<Py<PyAny>>,
}

#[pymethods]
impl PyKeyIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<Py<PyAny>> {
        self.keys.next()
    }
}

/// A YAML mapping, with the `dict` interface.
///
/// Keys may be `str`, `int`, `float`, `bool` or `None`; keys come back as the
/// Python value they resolve to. Values come back as `Node` objects, which
/// compare equal to the Python values they hold.
#[pyclass(name = "Mapping", unsendable, module = "yaml_edit._yaml_edit")]
pub(crate) struct PyMapping {
    pub(crate) inner: Mapping,
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

    fn contains_key(&self, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        contains(&self.inner, key)
    }

    fn __contains__(&self, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        contains(&self.inner, key)
    }

    /// The value node for `key`, or `default` if absent.
    #[pyo3(signature = (key, default=None))]
    fn get(
        &self,
        py: Python<'_>,
        key: &Bound<'_, PyAny>,
        default: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        get_or(py, &self.inner, key, default)
    }

    fn __getitem__(&self, key: &Bound<'_, PyAny>) -> PyResult<PyNode> {
        getitem(&self.inner, key)
    }

    /// The value for `key` if it is a mapping, or `None`.
    fn get_mapping(&self, key: &Bound<'_, PyAny>) -> PyResult<Option<PyMapping>> {
        let key = ValueArg::extract(key)?;
        Ok(call_with_value!(&key, |k| self.inner.get_mapping(k)).map(|inner| PyMapping { inner }))
    }

    /// The value for `key` if it is a sequence, or `None`.
    fn get_sequence(&self, key: &Bound<'_, PyAny>) -> PyResult<Option<PySequence>> {
        let key = ValueArg::extract(key)?;
        Ok(
            call_with_value!(&key, |k| self.inner.get_sequence(k))
                .map(|inner| PySequence { inner }),
        )
    }

    /// Set `key` to `value`, updating an existing entry in place.
    ///
    /// A new key is appended, or with `field_order` (a list of keys) placed
    /// after the keys that precede it in that order.
    #[pyo3(signature = (key, value, *, field_order=None))]
    fn set(
        &self,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
        field_order: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        set(&self.inner, key, value, field_order)
    }

    fn __setitem__(&self, key: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        set(&self.inner, key, value, None)
    }

    /// Remove `key`, returning `True` if it was present.
    fn remove(&self, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        remove(&self.inner, key)
    }

    fn __delitem__(&self, key: &Bound<'_, PyAny>) -> PyResult<()> {
        delitem(&self.inner, key)
    }

    /// Remove `key` and return its value node, or `default` if absent.
    ///
    /// Raises `KeyError` if `key` is absent and no default is given.
    #[pyo3(signature = (key, *default))]
    fn pop(
        &self,
        py: Python<'_>,
        key: &Bound<'_, PyAny>,
        default: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        pop(py, &self.inner, key, default)
    }

    /// Remove and return the last `(key, value)` pair.
    fn popitem(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, Py<PyAny>)> {
        popitem(py, &self.inner)
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
        setdefault(&self.inner, key, default.unwrap_or(&none))
    }

    /// Set keys from a mapping or iterable of pairs, and keyword arguments.
    #[pyo3(signature = (other=None, **kwargs))]
    fn update(
        &self,
        other: Option<&Bound<'_, PyAny>>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        update(&self.inner, other, kwargs)
    }

    /// Rename `old_key` to `new_key`, returning `True` on success.
    fn rename_key(&self, old_key: &Bound<'_, PyAny>, new_key: &Bound<'_, PyAny>) -> PyResult<bool> {
        rename_key(&self.inner, old_key, new_key)
    }

    fn clear(&self) {
        self.inner.clear();
    }

    /// The keys, in document order.
    fn keys(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        keys(py, &self.inner)
    }

    /// The value nodes, in document order.
    fn values(&self) -> Vec<PyNode> {
        values(&self.inner)
    }

    /// `(key, value node)` pairs, in document order.
    fn items(&self, py: Python<'_>) -> PyResult<Vec<(Py<PyAny>, PyNode)>> {
        items(py, &self.inner)
    }

    fn __iter__(&self, py: Python<'_>) -> PyResult<PyKeyIterator> {
        Ok(PyKeyIterator {
            keys: keys(py, &self.inner)?.into_iter(),
        })
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
    fn find_entry(&self, key: &Bound<'_, PyAny>) -> PyResult<Option<PyEntry>> {
        let flow_context = self.inner.is_flow_style();
        let key = ValueArg::extract(key)?;
        Ok(
            call_with_value!(&key, |k| self.inner.find_entry_by_key(k)).map(|inner| PyEntry {
                inner,
                flow_context,
            }),
        )
    }

    /// Every entry whose key matches `key`, in document order.
    ///
    /// YAML permits repeated keys; this returns one handle per occurrence.
    fn find_all_entries(&self, key: &Bound<'_, PyAny>) -> PyResult<Vec<PyEntry>> {
        let flow_context = self.inner.is_flow_style();
        let key = ValueArg::extract(key)?;
        let entries: Vec<MappingEntry> =
            call_with_value!(&key, |k| self.inner.find_all_entries_by_key(k).collect());
        Ok(entries
            .into_iter()
            .map(|inner| PyEntry {
                inner,
                flow_context,
            })
            .collect())
    }

    /// The number of entries whose key matches `key`.
    fn count_key(&self, key: &Bound<'_, PyAny>) -> PyResult<usize> {
        let key = ValueArg::extract(key)?;
        Ok(call_with_value!(&key, |k| self
            .inner
            .find_all_entries_by_key(k)
            .count()))
    }

    /// Remove the `n`-th (0-based) occurrence of `key`, returning `True` if it
    /// existed.
    fn remove_nth(&self, key: &Bound<'_, PyAny>, n: usize) -> PyResult<bool> {
        let key = ValueArg::extract(key)?;
        Ok(call_with_value!(&key, |k| self.inner.remove_nth_occurrence(k, n)).is_some())
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
    /// `False` if `after_key` is not present.
    fn insert_after(
        &self,
        after_key: &Bound<'_, PyAny>,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        place(&self.inner, Placement::InsertAfter, after_key, key, value)
    }

    /// Insert `key` immediately before `before_key`.
    ///
    /// An existing `key` is updated in place rather than moved. Returns
    /// `False` if `before_key` is not present.
    fn insert_before(
        &self,
        before_key: &Bound<'_, PyAny>,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        place(&self.inner, Placement::InsertBefore, before_key, key, value)
    }

    /// Set `key` and place it immediately after `after_key`, moving it if it
    /// already exists. Returns `False` if `after_key` is not present.
    fn move_after(
        &self,
        after_key: &Bound<'_, PyAny>,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        place(&self.inner, Placement::MoveAfter, after_key, key, value)
    }

    /// Set `key` and place it immediately before `before_key`, moving it if
    /// it already exists. Returns `False` if `before_key` is not present.
    fn move_before(
        &self,
        before_key: &Bound<'_, PyAny>,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        place(&self.inner, Placement::MoveBefore, before_key, key, value)
    }

    /// Insert `key` at position `index`, or at the end if `index` is past it.
    ///
    /// An existing `key` is updated in place rather than moved.
    fn insert_at_index(
        &self,
        index: usize,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let key = ValueArg::extract(key)?;
        let value = ValueArg::extract(value)?;
        call_with_key_value!(&key, &value, |k, v| self.inner.insert_at_index(index, k, v));
        Ok(())
    }

    /// Reorder entries so the keys in `order` come first, in that order.
    ///
    /// Keys not in `order` follow, in their original relative order.
    fn reorder_fields(&self, order: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.reorder_fields(field_order(order)?);
        Ok(())
    }

    /// A read-only view resolving aliases and merge keys (`<<`) against the
    /// anchors of the enclosing document.
    fn merged(&self) -> PyResult<PyMergedMapping> {
        let registry = registry_for(&YamlNode::Mapping(self.inner.clone()));
        // Resolve once now so that a broken alias raises here, not on first use.
        MergedMapping::new(self.inner.clone(), &registry).map_err(resolve_error)?;
        Ok(PyMergedMapping {
            base: self.inner.clone(),
            registry,
        })
    }

    /// Convert to a `dict` of plain Python objects, resolving aliases and
    /// merge keys.
    fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, &YamlNode::Mapping(self.inner.clone()))
    }

    /// The `(start, end)` byte offsets of this mapping in the source text.
    fn byte_range(&self) -> (u32, u32) {
        span(self.inner.byte_range())
    }

    /// The 1-based `(line, column)` where this mapping starts.
    fn start_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().start)
    }

    /// The 1-based `(line, column)` where this mapping ends.
    fn end_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().end)
    }

    /// Compare by YAML value, ignoring formatting and key order.
    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        compare(py, &self.inner, other)
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Mapping len={}>", self.inner.len())
    }
}

/// Where `place` puts a key relative to an existing one.
pub(crate) enum Placement {
    InsertAfter,
    InsertBefore,
    MoveAfter,
    MoveBefore,
}

/// Set `key` to `value` next to `anchor`, returning `False` if `anchor` is
/// not in the mapping.
pub(crate) fn place(
    mapping: &Mapping,
    placement: Placement,
    anchor: &Bound<'_, PyAny>,
    key: &Bound<'_, PyAny>,
    value: &Bound<'_, PyAny>,
) -> PyResult<bool> {
    let anchor = ValueArg::extract(anchor)?;
    let key = ValueArg::extract(key)?;
    let value = ValueArg::extract(value)?;
    Ok(call_with_value!(&anchor, |a| call_with_key_value!(
        &key,
        &value,
        |k, v| match placement {
            Placement::InsertAfter => mapping.insert_after(a, k, v),
            Placement::InsertBefore => mapping.insert_before(a, k, v),
            Placement::MoveAfter => mapping.move_after(a, k, v),
            Placement::MoveBefore => mapping.move_before(a, k, v),
        }
    )))
}

/// A read-only view of a mapping with aliases and merge keys (`<<`) resolved.
///
/// Obtained from `Document.merged()` or `Mapping.merged()`. Direct entries take
/// precedence over keys contributed by merge sources, the synthetic `<<` key is
/// hidden, and aliases (`*name`) are expanded to their anchored values.
///
/// The view does not mutate the document; mutate through the original
/// `Mapping`/`Document` instead.
///
/// Unlike the other collection types, this has no `__str__`: a merged view is
/// synthetic and has no single source span, so serializing it would have to
/// emit the un-merged source and hide the merged-in keys. Read the resolved
/// values through `items`/`get` instead.
#[pyclass(name = "MergedMapping", unsendable, module = "yaml_edit._yaml_edit")]
pub(crate) struct PyMergedMapping {
    pub(crate) base: Mapping,
    pub(crate) registry: AnchorRegistry,
}

impl PyMergedMapping {
    /// Borrow `base` and `registry` as a `MergedMapping` for a single query.
    ///
    /// The view is rebuilt on every query so that it reflects later edits to
    /// the document, which can also make it fail to resolve.
    fn view(&self) -> PyResult<MergedMapping<'_>> {
        MergedMapping::new(self.base.clone(), &self.registry).map_err(resolve_error)
    }

    fn lookup(&self, key: &Bound<'_, PyAny>) -> PyResult<Option<YamlNode>> {
        let key = ValueArg::extract(key)?;
        let view = self.view()?;
        Ok(call_with_value!(&key, |k| view.get(k)))
    }

    fn keys_vec(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        self.view()?
            .keys()
            .map(|key| to_python_with(py, &key, &self.registry))
            .collect()
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

    fn contains_key(&self, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(self.lookup(key)?.is_some())
    }

    fn __contains__(&self, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(self.lookup(key)?.is_some())
    }

    /// The value for `key`, resolving aliases and merge keys, or `default`.
    #[pyo3(signature = (key, default=None))]
    fn get(
        &self,
        py: Python<'_>,
        key: &Bound<'_, PyAny>,
        default: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        match self.lookup(key)? {
            Some(inner) => node(py, inner),
            None => Ok(default.unwrap_or_else(|| py.None())),
        }
    }

    fn __getitem__(&self, key: &Bound<'_, PyAny>) -> PyResult<PyNode> {
        self.lookup(key)?
            .map(|inner| PyNode { inner })
            .ok_or_else(|| PyKeyError::new_err(key.clone().unbind()))
    }

    /// Look up a nested mapping, returning another merged view that resolves
    /// against the same anchors.
    fn get_merged(&self, key: &Bound<'_, PyAny>) -> PyResult<Option<PyMergedMapping>> {
        let key = ValueArg::extract(key)?;
        let view = self.view()?;
        let nested = call_with_value!(&key, |k| view.get_merged(k)).map_err(resolve_error)?;
        Ok(nested.map(|nested| PyMergedMapping {
            base: nested.base().clone(),
            registry: self.registry.clone(),
        }))
    }

    /// The merged keys, in resolution order (direct entries first).
    fn keys(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        self.keys_vec(py)
    }

    /// The merged value nodes, in resolution order.
    fn values(&self) -> PyResult<Vec<PyNode>> {
        Ok(self
            .view()?
            .values()
            .map(|inner| PyNode { inner })
            .collect())
    }

    /// `(key, value node)` pairs in the merged view, in resolution order.
    fn items(&self, py: Python<'_>) -> PyResult<Vec<(Py<PyAny>, PyNode)>> {
        self.view()?
            .iter()
            .map(|(key, inner)| Ok((to_python_with(py, &key, &self.registry)?, PyNode { inner })))
            .collect()
    }

    fn __iter__(&self, py: Python<'_>) -> PyResult<PyKeyIterator> {
        Ok(PyKeyIterator {
            keys: self.keys_vec(py)?.into_iter(),
        })
    }

    /// Convert to a `dict` of plain Python objects.
    fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python_with(py, &YamlNode::Mapping(self.base.clone()), &self.registry)
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "<yaml_edit.MergedMapping len={}>",
            self.view()?.len()
        ))
    }
}

/// Resolve a possibly negative index against `len`, like `list` does.
pub(crate) fn resolve_index(index: isize, len: usize) -> Option<usize> {
    let index = if index < 0 {
        index.checked_add_unsigned(len)?
    } else {
        index
    };
    usize::try_from(index).ok().filter(|&i| i < len)
}

fn index_error() -> PyErr {
    PyIndexError::new_err("sequence index out of range")
}

/// Clamp a possibly negative index to `0..=len`, as `list.insert` and
/// `list.index` do.
fn clamp_index(index: isize, len: usize) -> usize {
    if index < 0 {
        index
            .checked_add_unsigned(len)
            .map_or(0, |i| i.max(0) as usize)
    } else {
        (index as usize).min(len)
    }
}

/// A YAML sequence, with the `list` interface.
///
/// Items come back as `Node` objects, which compare equal to the Python values
/// they hold.
#[pyclass(
    name = "Sequence",
    sequence,
    unsendable,
    module = "yaml_edit._yaml_edit"
)]
pub(crate) struct PySequence {
    pub(crate) inner: Sequence,
}

impl PySequence {
    fn position(
        &self,
        value: &Bound<'_, PyAny>,
        start: usize,
        stop: usize,
    ) -> PyResult<Option<usize>> {
        let value = ValueArg::extract(value)?;
        Ok(self
            .inner
            .values()
            .enumerate()
            .skip(start)
            .take(stop.saturating_sub(start))
            .find(|(_, item)| call_with_value!(&value, |v| yaml_edit::yaml_eq(item, v)))
            .map(|(i, _)| i))
    }

    fn append_value(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = ValueArg::extract(value)?;
        call_with_value!(&value, |v| self.inner.push(v));
        Ok(())
    }
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

    /// The item at `index` (negative counts from the end), or `None` if out
    /// of range.
    fn get(&self, index: isize) -> Option<PyNode> {
        resolve_index(index, self.inner.len())
            .and_then(|i| self.inner.get(i))
            .map(|inner| PyNode { inner })
    }

    /// The item at an index, or a list of items for a slice.
    fn __getitem__(&self, py: Python<'_>, index: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(slice) = index.cast::<PySlice>() {
            let len = self.inner.len();
            let indices = slice.indices(len.try_into()?)?;
            let items: Vec<YamlNode> = self.inner.values().collect();
            let nodes = (0..indices.slicelength)
                .map(|i| {
                    let i = indices.start + indices.step * i as isize;
                    node(py, items[i as usize].clone())
                })
                .collect::<PyResult<Vec<_>>>()?;
            return Ok(pyo3::types::PyList::new(py, nodes)?.into_any().unbind());
        }
        let index: isize = index.extract()?;
        let inner = resolve_index(index, self.inner.len())
            .and_then(|i| self.inner.get(i))
            .ok_or_else(index_error)?;
        node(py, inner)
    }

    fn first(&self) -> Option<PyNode> {
        self.inner.first().map(|inner| PyNode { inner })
    }

    fn last(&self) -> Option<PyNode> {
        self.inner.last().map(|inner| PyNode { inner })
    }

    /// Append `value` to the end of the sequence.
    fn append(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.append_value(value)
    }

    /// Append every item of `values`.
    fn extend(&self, values: &Bound<'_, PyAny>) -> PyResult<()> {
        for value in values.try_iter()? {
            self.append_value(&value?)?;
        }
        Ok(())
    }

    /// `seq += values` appends every item of `values`.
    fn __iadd__(&self, values: &Bound<'_, PyAny>) -> PyResult<()> {
        self.extend(values)
    }

    /// Reverse the items in place.
    fn reverse(&self) {
        let items: Vec<YamlNode> = self.inner.values().collect();
        for (i, item) in items.iter().rev().enumerate() {
            self.inner.set(i, item);
        }
    }

    /// Deprecated alias of `append`.
    fn push(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        deprecated(py, c"Sequence.push() is deprecated; use append()")?;
        self.append_value(value)
    }

    /// Insert `value` before `index`, like `list.insert`.
    fn insert(&self, index: isize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let index = clamp_index(index, self.inner.len());
        let value = ValueArg::extract(value)?;
        call_with_value!(&value, |v| self.inner.insert(index, v));
        Ok(())
    }

    /// Replace the item at `index`, returning `False` if it is out of range.
    fn set(&self, index: isize, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        let Some(index) = resolve_index(index, self.inner.len()) else {
            return Ok(false);
        };
        let value = ValueArg::extract(value)?;
        Ok(call_with_value!(&value, |v| self.inner.set(index, v)))
    }

    fn __setitem__(&self, index: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        if index.is_instance_of::<PySlice>() {
            // TODO: support slice assignment.
            return Err(PyTypeError::new_err("slice assignment is not supported"));
        }
        let index = resolve_index(index.extract()?, self.inner.len()).ok_or_else(index_error)?;
        let value = ValueArg::extract(value)?;
        call_with_value!(&value, |v| self.inner.set(index, v));
        Ok(())
    }

    fn __delitem__(&self, index: &Bound<'_, PyAny>) -> PyResult<()> {
        let len = self.inner.len();
        if let Ok(slice) = index.cast::<PySlice>() {
            let indices = slice.indices(len.try_into()?)?;
            let mut doomed: Vec<usize> = (0..indices.slicelength)
                .map(|i| (indices.start + indices.step * i as isize) as usize)
                .collect();
            doomed.sort_unstable_by(|a, b| b.cmp(a));
            for i in doomed {
                self.inner.remove(i);
            }
            return Ok(());
        }
        let index = resolve_index(index.extract()?, len).ok_or_else(index_error)?;
        self.inner.remove(index);
        Ok(())
    }

    /// Remove the first item equal to `value`, like `list.remove`.
    ///
    /// Raises `ValueError` if there is no such item. Passing an `int` is
    /// deprecated: it removes and returns the item at that index, as in
    /// earlier versions; use `pop(index)` or `del seq[index]` for that.
    fn remove(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Option<PyNode>> {
        if value.is_instance_of::<PyInt>() && !value.is_instance_of::<PyBool>() {
            deprecated(
                py,
                c"Sequence.remove(index) is deprecated; use pop(index) or del seq[index]. \
                  It will remove by value, like list.remove, in a future release",
            )?;
            let index: usize = value.extract()?;
            return Ok(self.inner.remove(index).map(|inner| PyNode { inner }));
        }
        let index = self
            .position(value, 0, self.inner.len())?
            .ok_or_else(|| PyValueError::new_err("Sequence.remove(x): x not in sequence"))?;
        self.inner.remove(index);
        Ok(None)
    }

    /// Remove and return the item at `index` (the last by default).
    #[pyo3(signature = (index=-1))]
    fn pop(&self, index: isize) -> PyResult<PyNode> {
        let len = self.inner.len();
        if len == 0 {
            return Err(PyIndexError::new_err("pop from empty sequence"));
        }
        let index = resolve_index(index, len)
            .ok_or_else(|| PyIndexError::new_err("pop index out of range"))?;
        self.inner
            .remove(index)
            .map(|inner| PyNode { inner })
            .ok_or_else(index_error)
    }

    /// The index of the first item equal to `value`.
    ///
    /// Raises `ValueError` if there is none.
    #[pyo3(signature = (value, start=0, stop=None))]
    fn index(
        &self,
        value: &Bound<'_, PyAny>,
        start: isize,
        stop: Option<isize>,
    ) -> PyResult<usize> {
        let len = self.inner.len();
        let start = clamp_index(start, len);
        let stop = stop.map_or(len, |stop| clamp_index(stop, len));
        self.position(value, start, stop)?
            .ok_or_else(|| PyValueError::new_err("value is not in sequence"))
    }

    /// The number of items equal to `value`.
    fn count(&self, value: &Bound<'_, PyAny>) -> PyResult<usize> {
        let value = ValueArg::extract(value)?;
        Ok(self
            .inner
            .values()
            .filter(|item| call_with_value!(&value, |v| yaml_edit::yaml_eq(item, v)))
            .count())
    }

    fn __contains__(&self, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        match ValueArg::extract_for_comparison(value)? {
            Some(value) => Ok(self
                .inner
                .values()
                .any(|item| call_with_value!(&value, |v| yaml_edit::yaml_eq(&item, v)))),
            None => Ok(false),
        }
    }

    fn clear(&self) {
        self.inner.clear();
    }

    /// The item nodes, in order.
    fn values(&self) -> Vec<PyNode> {
        self.inner.values().map(|inner| PyNode { inner }).collect()
    }

    fn __iter__(&self) -> PyNodeIterator {
        PyNodeIterator {
            nodes: self.values().into_iter(),
        }
    }

    /// Convert to a `list` of plain Python objects, resolving aliases.
    fn to_python(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, &YamlNode::Sequence(self.inner.clone()))
    }

    /// The `(start, end)` byte offsets of this sequence in the source text.
    fn byte_range(&self) -> (u32, u32) {
        span(self.inner.byte_range())
    }

    /// The 1-based `(line, column)` where this sequence starts.
    fn start_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().start)
    }

    /// The 1-based `(line, column)` where this sequence ends.
    fn end_position(&self, source_text: &str) -> (usize, usize) {
        line_column(source_text, self.inner.byte_range().end)
    }

    /// Compare by YAML value, ignoring formatting.
    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        compare(py, &self.inner, other)
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("<yaml_edit.Sequence len={}>", self.inner.len())
    }
}

/// Iterator over a sequence's item nodes, returned by `Sequence.__iter__`.
#[pyclass(unsendable, module = "yaml_edit._yaml_edit")]
pub(crate) struct PyNodeIterator {
    pub(crate) nodes: std::vec::IntoIter<PyNode>,
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
