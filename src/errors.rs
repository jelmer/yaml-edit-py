//! Python exception types, and conversions from the crate's error types.

use std::path::Path;

use pyo3::create_exception;
use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;

create_exception!(
    yaml_edit,
    YamlError,
    PyValueError,
    "Base class for the errors yaml_edit raises about YAML content."
);
create_exception!(
    yaml_edit,
    ParseError,
    YamlError,
    "The text is not valid YAML. `line` and `column` give the location, if known."
);
create_exception!(
    yaml_edit,
    PathError,
    YamlError,
    "A dotted path is malformed or cannot be followed."
);
create_exception!(
    yaml_edit,
    ResolveError,
    YamlError,
    "An alias or merge key cannot be resolved."
);

/// Convert a crate error: I/O failures become `OSError`, parse failures
/// `ParseError` and anything else `YamlError`.
pub(crate) fn yaml_error(err: yaml_edit::YamlError) -> PyErr {
    let message = err.to_string();
    match err {
        yaml_edit::YamlError::Io(e) => e.into(),
        yaml_edit::YamlError::Parse { line, column, .. } => Python::attach(|py| {
            let err = ParseError::new_err(message);
            let value = err.value(py);
            match value
                .setattr("line", line)
                .and_then(|()| value.setattr("column", column))
            {
                Ok(()) => err,
                Err(e) => e,
            }
        }),
        _ => YamlError::new_err(message),
    }
}

/// Convert an I/O error on `path` to the `OSError` subclass Python would
/// raise, such as `FileNotFoundError`, with `filename` set.
pub(crate) fn io_error(err: std::io::Error, path: &Path) -> PyErr {
    let filename = path.to_string_lossy().into_owned();
    match err.raw_os_error() {
        Some(code) => {
            let message = std::io::Error::from_raw_os_error(code).to_string();
            let suffix = format!(" (os error {code})");
            let message = message
                .strip_suffix(&suffix)
                .unwrap_or(&message)
                .to_string();
            PyOSError::new_err((code, message, filename))
        }
        None => PyOSError::new_err(format!("{filename}: {err}")),
    }
}

/// Like `yaml_error`, naming `path` in I/O errors.
pub(crate) fn file_error(err: yaml_edit::YamlError, path: &Path) -> PyErr {
    match err {
        yaml_edit::YamlError::Io(e) => io_error(e, path),
        err => yaml_error(err),
    }
}

pub(crate) fn path_error(err: yaml_edit::path::PathError) -> PyErr {
    PathError::new_err(err.to_string())
}

pub(crate) fn resolve_error(err: yaml_edit::anchor_resolution::ResolveError) -> PyErr {
    ResolveError::new_err(err.to_string())
}
