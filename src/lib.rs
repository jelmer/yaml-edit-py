//! Python bindings for the `yaml-edit` crate.
//!
//! The wrapper types in `yaml-edit` are lightweight views over a shared rowan
//! syntax tree. rowan uses `Rc` internally, so the wrappers are neither `Send`
//! nor `Sync`; every `#[pyclass]` here is declared `unsendable`, which makes
//! PyO3 enforce single-thread access at runtime.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBool;
use yaml_edit::path::{AliasPolicy, PathError as CratePathError, YamlPath};
use yaml_edit::{byte_offset_to_line_column, AsYaml, TextPosition, YamlNode};

mod collections;
mod convert;
mod document;
mod errors;
mod nodes;

use convert::{call_with_value, to_python, ValueArg};

/// Convert a byte range to a Python `(start, end)` tuple.
pub(crate) fn span(range: TextPosition) -> (u32, u32) {
    (range.start, range.end)
}

/// The 1-based `(line, column)` of a byte offset in `source_text`.
pub(crate) fn line_column(source_text: &str, offset: u32) -> (usize, usize) {
    let lc = byte_offset_to_line_column(source_text, offset as usize);
    (lc.line, lc.column)
}

/// Compare a YAML value with a Python object by YAML semantics, or return
/// `NotImplemented` if the object has no YAML equivalent.
pub(crate) fn compare(
    py: Python<'_>,
    value: &impl AsYaml,
    other: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    Ok(match ValueArg::extract_for_comparison(other)? {
        Some(other) => PyBool::new(
            py,
            call_with_value!(&other, |o| yaml_edit::yaml_eq(value, o)),
        )
        .to_owned()
        .into_any()
        .unbind(),
        None => py.NotImplemented(),
    })
}

/// Hash a node as its Python value, so that a node and the Python value it
/// compares equal to hash alike. Collections are unhashable, as in Python.
pub(crate) fn python_hash(py: Python<'_>, node: &YamlNode) -> PyResult<isize> {
    to_python(py, node)?.bind(py).hash()
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
/// document is empty) and raising `PathError` for any other failure.
pub(crate) fn get_path(
    target: &impl YamlPath,
    path: &str,
    alias_policy: &str,
) -> PyResult<Option<nodes::PyNode>> {
    match target.try_get_path_with(path, parse_alias_policy(alias_policy)?) {
        Ok(inner) => Ok(Some(nodes::PyNode { inner })),
        Err(CratePathError::NotFound { .. } | CratePathError::NoRoot) => Ok(None),
        Err(e) => Err(errors::path_error(e)),
    }
}

/// Set `path` to `value`, creating intermediate mappings as needed.
pub(crate) fn set_path(
    target: &impl YamlPath,
    path: &str,
    value: &Bound<'_, PyAny>,
    alias_policy: &str,
) -> PyResult<()> {
    let policy = parse_alias_policy(alias_policy)?;
    let value = ValueArg::extract(value)?;
    call_with_value!(&value, |v| target.try_set_path_with(path, v, policy))
        .map_err(errors::path_error)
}

/// Remove `path`, returning `False` if the target does not exist (or the
/// document is empty) and raising `PathError` for any other failure.
pub(crate) fn remove_path(
    target: &impl YamlPath,
    path: &str,
    alias_policy: &str,
) -> PyResult<bool> {
    match target.try_remove_path_with(path, parse_alias_policy(alias_policy)?) {
        Ok(_) => Ok(true),
        Err(CratePathError::NotFound { .. } | CratePathError::NoRoot) => Ok(false),
        Err(e) => Err(errors::path_error(e)),
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
    let py = m.py();
    m.add_class::<document::PyDocument>()?;
    m.add_class::<document::PyYamlFile>()?;
    m.add_class::<document::PyDirective>()?;
    m.add_class::<document::PyViolation>()?;
    m.add_class::<document::PySchemaViolation>()?;
    m.add_class::<collections::PyMapping>()?;
    m.add_class::<collections::PyMergedMapping>()?;
    m.add_class::<collections::PySequence>()?;
    m.add_class::<nodes::PyNode>()?;
    m.add_class::<nodes::PyScalar>()?;
    m.add_class::<nodes::PyScalarValue>()?;
    m.add_class::<nodes::PyEntry>()?;
    m.add_class::<nodes::PyAlias>()?;
    m.add_class::<nodes::PyTaggedNode>()?;
    m.add_class::<nodes::PyComment>()?;
    m.add("YamlError", py.get_type::<errors::YamlError>())?;
    m.add("ParseError", py.get_type::<errors::ParseError>())?;
    m.add("PathError", py.get_type::<errors::PathError>())?;
    m.add("ResolveError", py.get_type::<errors::ResolveError>())?;
    m.add_function(wrap_pyfunction!(py_byte_offset_to_line_column, m)?)?;
    Ok(())
}
