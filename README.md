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

`Document`, `Mapping` and `Sequence` behave like `dict` and `list`, and edits
preserve the surrounding formatting and comments:

```python
from yaml_edit import Document

doc = Document.parse("# project config\nname: demo   # the name\ntags:\n  - a\n  - b\n")
doc["name"] = "renamed"
doc["tags"].as_sequence().append("c")
print(doc, end="")
# # project config
# name: renamed   # the name
# tags:
#   - a
#   - b
#   - c
```

Values come back as `Node` objects, which keep their place in the document but
compare equal to the Python values they hold. `to_python()` converts a node, or
a whole document, to plain Python objects:

```python
doc = Document.parse("port: 8080\nhosts: [a, b]\n1: one\n")
print(doc["port"] == 8080, doc["hosts"] == ["a", "b"])
# True True
print(doc.keys())
# ['port', 'hosts', 1]
print(doc.to_python())
# {'port': 8080, 'hosts': ['a', 'b'], 1: 'one'}
```

Keys may be strings, numbers, booleans or `None`. Values may be any of those,
or a `dict`, `list` or `tuple`, which become block mappings and sequences:

```python
doc = Document.parse("name: demo\n")
doc["server"] = {"host": "localhost", "ports": [80, 443]}
print(doc, end="")
# name: demo
# server:
#   host: localhost
#   ports:
#   - 80
#   - 443
```

The rest of the `dict` and `list` interfaces work too: `get` with a default,
`pop`, `setdefault`, `update`, negative indices and slices, `insert`,
`extend`, `index`, `count`, `remove` and `reverse`. `Mapping` and `Sequence`
are registered as `collections.abc.MutableMapping` and `MutableSequence`.

New keys can be placed relative to existing ones instead of being appended:

```python
doc = Document.parse("name: demo\nversion: 1.0\n")
doc.insert_after("name", "description", "A demo")
doc.reorder_fields(["version"])
print(doc, end="")
# version: 1.0
# name: demo
# description: A demo
```

Dotted paths reach into nested structures, creating intermediate mappings
when setting:

```python
doc = Document.parse("server:\n  host: localhost\n  ports:\n    - 80\n")
print(doc.get_path("server.ports[0]"))
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
`PathError`. Writing through an alias is refused unless you pass
`alias_policy="follow"` (modify the anchored node) or `alias_policy="expand"`
(replace the alias with a copy first).

To control how a scalar is written, pass a `ScalarValue`; to refer to an
anchor, pass an `Alias`:

```python
from yaml_edit import Alias, ScalarValue

doc = Document.parse("base: &b\n  k: v\n")
doc["version"] = ScalarValue("1.0", style="double-quoted")
doc["ref"] = Alias("b")
print(doc, end="")
# base: &b
#   k: v
# version: "1.0"
# ref: *b
```

Multi-document streams behave as a sequence of documents:

```python
from yaml_edit import YamlFile

stream = YamlFile.parse("a: 1\n---\nb: 2\n")
for document in stream:
    print(document.keys())
# ['a']
# ['b']
print(stream[-1]["b"])
# 2
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
print(prod["timeout"])  # the direct entry wins over the merged one
# 60
```

The merged view is read-only; edit through the original `Document` or `Mapping`.

Source positions, comments, validation and error-tolerant parsing are
available for tools such as linters:

```python
src = "name: demo  # the name\nname: again\n"
doc = Document.parse(src)
print(doc["name"].start_position(src))  # 1-based (line, column)
# (1, 7)
print([c.content() for c in doc.comments()])
# ['the name']
print([(v.rule, v.location) for v in doc.validate()])
# [('duplicate_keys', '2:1')]
doc, errors = Document.parse_relaxed("a: [1, 2\n")
print(len(errors))
# 1
```

## Errors

Invalid YAML raises `ParseError`, with `line` and `column` attributes. A bad
dotted path raises `PathError`, and an undefined or recursive alias
`ResolveError`. All three derive from `YamlError`, which is a `ValueError`.
Reading or writing a file raises the usual `OSError` subclasses.

## License

Apache-2.0
