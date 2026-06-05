"""Lossless YAML parser and editor.

Parse YAML while preserving formatting, comments, and whitespace, then make
targeted edits that leave the rest of the document untouched.

    >>> from yaml_edit import Document
    >>> doc = Document.parse("name: old\\nversion: 1.0\\n")
    >>> doc.set("name", "new")
    >>> print(doc, end="")
    name: new
    version: 1.0
"""

from ._yaml_edit import (
    Document,
    Mapping,
    Node,
    Scalar,
    Sequence,
    YamlFile,
)

__all__ = [
    "Document",
    "Mapping",
    "Node",
    "Scalar",
    "Sequence",
    "YamlFile",
]
