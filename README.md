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
doc = Document.parse("# project config\nname: demo   # the name\ntags:\n  - a\n  - b\n")
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

Mappings and sequences also support the usual `dict` and `list` operations, so
you can use subscripting instead of `get`/`set`:

```python
doc = Document.parse("name: demo\nversion: 1.0.0\n")
mapping = doc.as_mapping()
mapping["name"] = "renamed"  # same as mapping.set("name", "renamed")
print(str(mapping["version"]))  # same as mapping.get("version")
# 1.0.0
print("name" in mapping)
# True
del mapping["version"]
for key in mapping:
    print(key)
# name
```

`Document` itself supports the same subscripting for top-level keys, and
`Sequence` supports indexing, assignment, and `del` by position.

Dotted paths reach into nested structures, creating intermediate mappings
when setting:

```python
doc = Document.parse("server:\n  host: localhost\n  ports:\n    - 80\n")
print(str(doc.get_path("server.ports[0]")))
# 80
doc.set_path("server.tls.enabled", True)
doc.remove_path("server.host")
print(doc, end="")
# server:
#   ports:
#     - 80
#   tls:
#     enabled: true
```

`get_path` returns `None` and `remove_path` returns `False` when the path does
not exist; a malformed path, or one that descends into a scalar, raises
`ValueError`. Writing through an alias is refused unless you pass
`alias_policy="follow"` (modify the anchored node) or `alias_policy="expand"`
(replace the alias with a copy first).

Multi-document streams:

```python
from yaml_edit import YamlFile

stream = YamlFile.parse("a: 1\n---\nb: 2\n")
for document in stream.documents():
    print(document.keys())
# ['a']
# ['b']
```

Resolving anchors and merge keys (`<<`) for reading:

```python
doc = Document.parse(
    "defaults: &d\n"
    "  timeout: 30\n"
    "  retries: 3\n"
    "prod:\n"
    "  <<: *d\n"
    "  host: prod.example.com\n"
    "  timeout: 60\n"
)
prod = doc.merged().get_merged("prod")
print(prod.keys())
# ['host', 'timeout', 'retries']
print(str(prod["timeout"]))  # the direct entry wins over the merged one
# 60
```

The merged view is read-only; edit through the original `Document` or `Mapping`.

## Supported value types

Values passed to `set`, `push`, and `insert` may be `str`, `int`, `float`,
`bool`, or `None` (written as `null`). They may also be YAML nodes: a `Mapping`
or `Sequence` (including new empty ones created with `Mapping()` or
`Sequence(flow=True)`), a `Scalar`, `Alias`, `TaggedNode`, or a `Node` returned
by a getter. Nodes are inserted as a copy.

```python
from yaml_edit import Alias, Mapping

doc = Document.parse("base: &b\n  k: v\n")
server = Mapping()
server["host"] = "localhost"
doc.set("server", server)
doc.set("ref", Alias("b"))
print(doc, end="")
# base: &b
#   k: v
# server:
#   host: localhost
# ref: *b
```

## License

Apache-2.0
