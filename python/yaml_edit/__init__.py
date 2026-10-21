"""Lossless YAML parser and editor.

Parse YAML while preserving formatting, comments, and whitespace, then make
targeted edits that leave the rest of the document untouched.

    >>> from yaml_edit import Document
    >>> doc = Document.parse("name: old\\nversion: 1.0\\n")
    >>> doc["name"] = "new"
    >>> print(doc, end="")
    name: new
    version: 1.0
"""

from collections.abc import Mapping as _Mapping
from collections.abc import MutableMapping as _MutableMapping
from collections.abc import MutableSequence as _MutableSequence
from collections.abc import Sequence as _Sequence

from ._yaml_edit import (
    Alias,
    Comment,
    Directive,
    Document,
    Entry,
    Mapping,
    MergedMapping,
    Node,
    ParseError,
    PathError,
    ResolveError,
    Scalar,
    ScalarValue,
    SchemaViolation,
    Sequence,
    TaggedNode,
    Violation,
    YamlError,
    YamlFile,
    byte_offset_to_line_column,
)

_MutableMapping.register(Mapping)
_Mapping.register(MergedMapping)
_MutableSequence.register(Sequence)
_Sequence.register(YamlFile)

__all__ = [
    "Alias",
    "Comment",
    "Directive",
    "Document",
    "Entry",
    "Mapping",
    "MergedMapping",
    "Node",
    "ParseError",
    "PathError",
    "ResolveError",
    "Scalar",
    "ScalarValue",
    "SchemaViolation",
    "Sequence",
    "TaggedNode",
    "Violation",
    "YamlError",
    "YamlFile",
    "byte_offset_to_line_column",
]
