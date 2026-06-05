# yaml-edit

Python bindings for the [yaml-edit](https://github.com/jelmer/yaml-edit) Rust
crate: a lossless YAML parser and editor that preserves formatting, comments,
and whitespace.

Unlike most YAML libraries, yaml-edit does not round-trip through a plain data
model. It keeps the original syntax tree, so edits change only the parts you
touch and leave everything else - indentation, comment placement, quoting style,
blank lines - exactly as it was.

## Installation

```console
$ pip install yaml-edit
```

A Rust toolchain is required to build from source.

## Usage

```python
from yaml_edit import Document

doc = Document.parse("name: old-project\nversion: 1.0.0\n")
doc.set("name", "new-project")
doc.set("version", "2.0.0")
print(doc, end="")
# name: new-project
# version: 2.0.0
```

Edits preserve surrounding formatting and comments:

```python
doc = Document.parse(
    "# project config\n"
    "name: demo   # the name\n"
    "tags:\n"
    "  - a\n"
    "  - b\n"
)
mapping = doc.as_mapping()
mapping.set("name", "renamed")
print(doc, end="")
# # project config
# name: renamed   # the name
# tags:
#   - a
#   - b
```

Working with nested structures:

```python
doc = Document.parse("servers:\n  - web\n  - db\n")
servers = doc.as_mapping().get_sequence("servers")
servers.push("cache")
print(doc, end="")
# servers:
#   - web
#   - db
#   - cache
```

Multi-document streams:

```python
from yaml_edit import YamlFile

stream = YamlFile.parse("a: 1\n---\nb: 2\n")
for document in stream.documents():
    print(document.keys())
# ['a']
# ['b']
```

## Supported value types

Scalar values passed to `set`, `push`, and `insert` may be `str`, `int`,
`float`, or `bool`. `None` is rejected because the underlying editor has no way
to emit a YAML null scalar.

## License

Apache-2.0
