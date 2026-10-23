import collections.abc
import copy
import os
import tempfile
import unittest

from yaml_edit import (
    Alias,
    Comment,
    Directive,
    Document,
    Entry,
    Mapping,
    MergedMapping,
    ParseError,
    PathError,
    ResolveError,
    Scalar,
    ScalarValue,
    Sequence,
    TaggedNode,
    YamlError,
    YamlFile,
    byte_offset_to_line_column,
)


class DocumentTests(unittest.TestCase):
    def test_parse_and_roundtrip(self):
        text = "name: demo\nversion: 1.0.0\n"
        doc = Document.parse(text)
        self.assertEqual(str(doc), text)

    def test_parse_error(self):
        with self.assertRaises(ValueError):
            Document.parse("a: [1, 2\n")

    def test_set_preserves_formatting(self):
        doc = Document.parse("name: old   # the name\nversion: 1.0.0\n")
        doc.set("name", "new")
        self.assertEqual(str(doc), "name: new   # the name\nversion: 1.0.0\n")

    def test_set_new_key(self):
        doc = Document.parse("name: demo\n")
        doc.set("version", "1.0.0")
        self.assertEqual(doc.get_string("version"), "1.0.0")

    def test_setitem_and_getitem(self):
        doc = Document.parse("name: demo\n")
        doc["name"] = "renamed"
        self.assertEqual(str(doc["name"]), "renamed")

    def test_getitem_missing_raises_keyerror(self):
        doc = Document.parse("name: demo\n")
        with self.assertRaises(KeyError):
            doc["missing"]

    def test_keys_and_contains(self):
        doc = Document.parse("a: 1\nb: 2\nc: 3\n")
        self.assertEqual(doc.keys(), ["a", "b", "c"])
        self.assertIn("b", doc)
        self.assertNotIn("z", doc)

    def test_remove(self):
        doc = Document.parse("a: 1\nb: 2\n")
        self.assertTrue(doc.remove("a"))
        self.assertFalse(doc.remove("a"))
        self.assertEqual(doc.keys(), ["b"])

    def test_delitem_missing_raises_keyerror(self):
        doc = Document.parse("a: 1\n")
        with self.assertRaises(KeyError):
            del doc["missing"]

    def test_rename_key(self):
        doc = Document.parse("old: 1\n")
        self.assertTrue(doc.rename_key("old", "new"))
        self.assertEqual(doc.keys(), ["new"])

    def test_typed_scalars(self):
        doc = Document()
        doc.set("count", 42)
        doc.set("ratio", 1.5)
        doc.set("enabled", True)
        self.assertEqual(doc.get("count").as_int(), 42)
        self.assertEqual(doc.get("ratio").as_float(), 1.5)
        self.assertEqual(doc.get("enabled").as_bool(), True)

    def test_bool_not_treated_as_int(self):
        doc = Document()
        doc.set("flag", True)
        self.assertEqual(str(doc["flag"]), "true")

    def test_none_is_null(self):
        doc = Document.parse("a: 1\n")
        doc.set("x", None)
        self.assertEqual(str(doc), "a: 1\nx: null\n")
        self.assertTrue(doc["x"].as_scalar().is_null())

    def test_unsupported_type_rejected(self):
        doc = Document()
        with self.assertRaises(TypeError):
            doc.set("x", object())
        with self.assertRaises(TypeError):
            doc.set("x", {1, 2})

    def test_set_on_sequence_root_raises(self):
        doc = Document.parse("- a\n")
        with self.assertRaises(TypeError):
            doc.set("x", 1)
        with self.assertRaises(TypeError):
            doc["x"] = 1
        self.assertEqual(str(doc), "- a\n")


class MappingTests(unittest.TestCase):
    def test_as_mapping(self):
        doc = Document.parse("a: 1\nb: 2\n")
        mapping = doc.as_mapping()
        self.assertIsInstance(mapping, Mapping)
        self.assertEqual(len(mapping), 2)

    def test_items(self):
        doc = Document.parse("a: 1\nb: 2\n")
        items = doc.as_mapping().items()
        self.assertEqual([k for k, _ in items], ["a", "b"])
        self.assertEqual([str(v) for _, v in items], ["1", "2"])

    def test_nested_sequence_mutation_visible_through_doc(self):
        doc = Document.parse("tags:\n  - a\n  - b\n")
        seq = doc.as_mapping().get_sequence("tags")
        seq.append("c")
        self.assertEqual(str(doc), "tags:\n  - a\n  - b\n  - c\n")

    def test_get_mapping(self):
        doc = Document.parse("outer:\n  inner: 1\n")
        inner = doc.as_mapping().get_mapping("outer")
        self.assertEqual(inner.keys(), ["inner"])

    def test_clear(self):
        doc = Document.parse("a: 1\nb: 2\n")
        mapping = doc.as_mapping()
        mapping.clear()
        self.assertTrue(mapping.is_empty())

    def test_iter_yields_keys(self):
        doc = Document.parse("a: 1\nb: 2\nc: 3\n")
        mapping = doc.as_mapping()
        self.assertEqual(list(mapping), ["a", "b", "c"])

    def test_iter_in_comprehension(self):
        doc = Document.parse("a: 1\nb: 2\n")
        self.assertEqual({k for k in doc.as_mapping()}, {"a", "b"})

    def test_iter_returns_self(self):
        doc = Document.parse("a: 1\nb: 2\n")
        it = iter(doc.as_mapping())
        self.assertIs(iter(it), it)
        self.assertEqual(next(it), "a")
        self.assertEqual(next(it), "b")
        with self.assertRaises(StopIteration):
            next(it)


class MappingEntryTests(unittest.TestCase):
    def test_entries_expose_each_pair(self):
        doc = Document.parse("a: 1\nb: 2\n")
        entries = doc.as_mapping().entries()
        self.assertEqual([str(e.key()) for e in entries], ["a", "b"])
        self.assertEqual([str(e.value()) for e in entries], ["1", "2"])

    def test_find_entry(self):
        doc = Document.parse("a: 1\nb: 2\n")
        entry = doc.as_mapping().find_entry("b")
        self.assertIsInstance(entry, Entry)
        self.assertEqual(str(entry.value()), "2")

    def test_find_entry_missing(self):
        doc = Document.parse("a: 1\n")
        self.assertIsNone(doc.as_mapping().find_entry("z"))

    def test_repeated_keys_found_individually(self):
        doc = Document.parse("ref: first\nref: second\nref: third\n")
        mapping = doc.as_mapping()
        self.assertEqual(mapping.count_key("ref"), 3)
        entries = mapping.find_all_entries("ref")
        self.assertEqual(
            [str(e.value()) for e in entries], ["first", "second", "third"]
        )

    def test_set_value_on_specific_occurrence(self):
        doc = Document.parse("ref: first\nref: second\n")
        entries = doc.as_mapping().find_all_entries("ref")
        entries[1].set_value("changed")
        self.assertEqual(str(doc), "ref: first\nref: changed\n")

    def test_remove_nth_occurrence(self):
        doc = Document.parse("ref: first\nref: second\nref: third\n")
        mapping = doc.as_mapping()
        self.assertTrue(mapping.remove_nth("ref", 1))
        self.assertEqual(str(doc), "ref: first\nref: third\n")

    def test_remove_nth_out_of_range(self):
        doc = Document.parse("ref: only\n")
        self.assertFalse(doc.as_mapping().remove_nth("ref", 5))

    def test_entry_remove(self):
        doc = Document.parse("a: 1\nb: 2\n")
        mapping = doc.as_mapping()
        mapping.find_entry("a").remove()
        self.assertEqual(mapping.keys(), ["b"])

    def test_key_matches_ignores_quoting(self):
        doc = Document.parse('"a": 1\n')
        entry = doc.as_mapping().find_entry("a")
        self.assertTrue(entry.key_matches("a"))

    def test_repr(self):
        doc = Document.parse("a: 1\n")
        entry = doc.as_mapping().find_entry("a")
        self.assertEqual(repr(entry), '<yaml_edit.Entry key="a">')


class SequenceTests(unittest.TestCase):
    def test_as_sequence_and_indexing(self):
        doc = Document.parse("- a\n- b\n- c\n")
        seq = doc.as_sequence()
        self.assertIsInstance(seq, Sequence)
        self.assertEqual(len(seq), 3)
        self.assertEqual(str(seq[0]), "a")
        self.assertEqual(str(seq[2]), "c")

    def test_index_out_of_range(self):
        doc = Document.parse("- a\n")
        with self.assertRaises(IndexError):
            doc.as_sequence()[5]

    def test_push_pop(self):
        doc = Document.parse("- a\n")
        seq = doc.as_sequence()
        seq.append("b")
        self.assertEqual(len(seq), 2)
        popped = seq.pop()
        self.assertEqual(str(popped), "b")
        self.assertEqual(len(seq), 1)

    def test_first_last(self):
        doc = Document.parse("- a\n- b\n- c\n")
        seq = doc.as_sequence()
        self.assertEqual(str(seq.first()), "a")
        self.assertEqual(str(seq.last()), "c")

    def test_values(self):
        doc = Document.parse("- 1\n- 2\n")
        seq = doc.as_sequence()
        self.assertEqual([n.as_int() for n in seq.values()], [1, 2])

    def test_iter(self):
        doc = Document.parse("- 1\n- 2\n- 3\n")
        seq = doc.as_sequence()
        self.assertEqual([n.as_int() for n in seq], [1, 2, 3])

    def test_iter_returns_self(self):
        doc = Document.parse("- 1\n- 2\n")
        it = iter(doc.as_sequence())
        self.assertIs(iter(it), it)
        self.assertEqual(next(it).as_int(), 1)
        self.assertEqual(next(it).as_int(), 2)
        with self.assertRaises(StopIteration):
            next(it)

    def test_delitem(self):
        doc = Document.parse("- a\n- b\n- c\n")
        seq = doc.as_sequence()
        del seq[1]
        self.assertEqual([str(n) for n in seq], ["a", "c"])

    def test_delitem_out_of_range(self):
        doc = Document.parse("- a\n")
        with self.assertRaises(IndexError):
            del doc.as_sequence()[5]


class ScalarTests(unittest.TestCase):
    def test_quoted(self):
        doc = Document.parse('name: "quoted"\n')
        scalar = doc["name"].as_scalar()
        self.assertIsInstance(scalar, Scalar)
        self.assertTrue(scalar.is_quoted())
        self.assertEqual(scalar.value(), '"quoted"')
        self.assertEqual(scalar.unquoted_value(), "quoted")

    def test_set_value_in_place(self):
        doc = Document.parse("name: old\n")
        doc["name"].as_scalar().set_value("new")
        self.assertEqual(doc.get_string("name"), "new")

    def test_block_scalar(self):
        doc = Document.parse("text: |\n  hi\nplain: x\n")
        self.assertTrue(doc["text"].as_scalar().is_block())
        self.assertFalse(doc["plain"].as_scalar().is_block())


class NodeTests(unittest.TestCase):
    def test_node_kinds(self):
        doc = Document.parse("m:\n  k: v\ns:\n  - 1\nx: scalar\n")
        mapping = doc.as_mapping()
        self.assertEqual(mapping.get("m").kind, "mapping")
        self.assertEqual(mapping.get("s").kind, "sequence")
        self.assertEqual(mapping.get("x").kind, "scalar")

    def test_node_views(self):
        doc = Document.parse("x: scalar\n")
        node = doc["x"]
        self.assertTrue(node.is_scalar())
        self.assertIsNotNone(node.as_scalar())
        self.assertIsNone(node.as_mapping())


class MergedMappingTests(unittest.TestCase):
    MERGE_YAML = (
        "defaults: &d\n"
        "  timeout: 30\n"
        "  retries: 3\n"
        "prod:\n"
        "  <<: *d\n"
        "  host: prod.example.com\n"
        "  timeout: 60\n"
    )

    def test_non_mapping_root_returns_none(self):
        self.assertIsNone(Document.parse("- 1\n- 2\n").merged())

    def test_merged_root_keys(self):
        merged = Document.parse(self.MERGE_YAML).merged()
        self.assertIsInstance(merged, MergedMapping)
        self.assertEqual(merged.keys(), ["defaults", "prod"])

    def test_merge_key_brings_in_entries(self):
        prod = Document.parse(self.MERGE_YAML).merged().get_merged("prod")
        self.assertEqual(prod.keys(), ["host", "timeout", "retries"])
        self.assertEqual(len(prod), 3)
        self.assertFalse(prod.is_empty())

    def test_direct_entry_overrides_merged(self):
        prod = Document.parse(self.MERGE_YAML).merged().get_merged("prod")
        self.assertEqual(str(prod["timeout"]), "60")

    def test_merged_in_entry_is_visible(self):
        prod = Document.parse(self.MERGE_YAML).merged().get_merged("prod")
        self.assertEqual(str(prod.get("retries")), "3")

    def test_merge_key_itself_is_hidden(self):
        prod = Document.parse(self.MERGE_YAML).merged().get_merged("prod")
        self.assertIsNone(prod.get("<<"))
        self.assertNotIn("<<", prod)

    def test_contains_and_missing(self):
        prod = Document.parse(self.MERGE_YAML).merged().get_merged("prod")
        self.assertIn("host", prod)
        self.assertTrue(prod.contains_key("host"))
        self.assertNotIn("absent", prod)
        self.assertFalse(prod.contains_key("absent"))
        self.assertIsNone(prod.get("absent"))

    def test_getitem_missing_raises_keyerror(self):
        prod = Document.parse(self.MERGE_YAML).merged().get_merged("prod")
        with self.assertRaises(KeyError):
            prod["absent"]

    def test_items_and_iter(self):
        prod = Document.parse(self.MERGE_YAML).merged().get_merged("prod")
        self.assertEqual(
            [(k, str(v)) for k, v in prod.items()],
            [("host", "prod.example.com"), ("timeout", "60"), ("retries", "3")],
        )
        self.assertEqual(list(prod), ["host", "timeout", "retries"])

    def test_values(self):
        prod = Document.parse(self.MERGE_YAML).merged().get_merged("prod")
        self.assertEqual(
            [str(v) for v in prod.values()],
            ["prod.example.com", "60", "3"],
        )

    def test_alias_value_resolves(self):
        doc = Document.parse("config: &cfg\n  port: 8080\nserver: *cfg\n")
        server = doc.merged().get_merged("server")
        self.assertEqual(str(server["port"]), "8080")

    def test_get_merged_missing_returns_none(self):
        merged = Document.parse(self.MERGE_YAML).merged()
        self.assertIsNone(merged.get_merged("absent"))

    def test_undefined_merge_alias_raises(self):
        doc = Document.parse("<<: *missing\nb: 1\n")
        with self.assertRaises(ValueError):
            doc.merged()

    def test_nested_undefined_merge_alias_raises(self):
        merged = Document.parse("a:\n  <<: *missing\n  b: 1\n").merged()
        with self.assertRaises(ValueError):
            merged.get_merged("a")

    def test_invalid_merge_operand_raises(self):
        merged = Document.parse("a:\n  <<: [1]\n  b: 1\n").merged()
        with self.assertRaises(ValueError):
            merged.get_merged("a")


class NodeValueTests(unittest.TestCase):
    def test_set_block_mapping(self):
        doc = Document.parse("a: 1\n")
        child = Mapping()
        child["x"] = 1
        child["y"] = "two"
        doc.set("child", child)
        self.assertEqual(str(doc), "a: 1\nchild:\n  x: 1\n  y: two\n")

    def test_set_flow_collections(self):
        doc = Document.parse("a: 1\n")
        doc.set("m", Mapping(flow=True))
        doc.set("s", Sequence(flow=True))
        self.assertEqual(str(doc), "a: 1\nm: {}\ns: []\n")

    def test_new_collections_are_empty(self):
        self.assertTrue(Mapping().is_empty())
        self.assertFalse(Mapping().is_flow_style())
        self.assertTrue(Mapping(flow=True).is_flow_style())
        self.assertEqual(len(Sequence()), 0)
        self.assertTrue(Sequence(flow=True).is_flow_style())

    def test_push_sequence_and_null(self):
        seq = Document.parse("- 1\n").as_sequence()
        seq.append(None)
        seq.append(Sequence(flow=True))
        self.assertEqual(str(seq), "- 1\n- null\n- []\n")

    def test_copy_node_value(self):
        doc = Document.parse("a:\n  k: v\n")
        doc.set("b", doc["a"])
        self.assertEqual(str(doc), "a:\n  k: v\nb:\n  k: v\n")

    def test_entry_set_value_node(self):
        doc = Document.parse("a: 1\n")
        doc.as_mapping().entries()[0].set_value(Sequence(flow=True))
        self.assertEqual(str(doc), "a: []\n")

    def test_set_alias(self):
        doc = Document.parse("base: &b\n  k: v\n")
        doc.set("ref", Alias("b"))
        self.assertEqual(str(doc), "base: &b\n  k: v\nref: *b\n")
        self.assertEqual(str(doc.merged()["ref"].as_mapping()["k"]), "v")


class AliasTests(unittest.TestCase):
    def test_alias_node(self):
        doc = Document.parse("base: &b 1\nref: *b\n")
        node = doc["ref"]
        self.assertEqual(node.kind, "alias")
        self.assertTrue(node.is_alias())
        alias = node.as_alias()
        self.assertIsInstance(alias, Alias)
        self.assertEqual(alias.name(), "b")
        self.assertEqual(str(alias), "*b")

    def test_non_alias(self):
        node = Document.parse("a: 1\n")["a"]
        self.assertFalse(node.is_alias())
        self.assertIsNone(node.as_alias())

    def test_new_alias(self):
        self.assertEqual(Alias("anchor").name(), "anchor")
        self.assertEqual(str(Alias("anchor")), "*anchor")


class TaggedNodeTests(unittest.TestCase):
    def test_tagged_scalar(self):
        node = Document.parse("t: !custom val\n")["t"]
        self.assertEqual(node.kind, "tagged")
        self.assertTrue(node.is_tagged())
        tagged = node.as_tagged()
        self.assertIsInstance(tagged, TaggedNode)
        self.assertEqual(tagged.tag(), "!custom")
        self.assertEqual(tagged.as_string(), "val")
        self.assertEqual(str(tagged.value()), "val")
        self.assertIsNone(tagged.as_mapping())
        self.assertEqual(str(tagged), "!custom val")

    def test_tagged_mapping(self):
        tagged = Document.parse("t: !thing\n  a: 1\n")["t"].as_tagged()
        self.assertEqual(tagged.tag(), "!thing")
        self.assertIsNone(tagged.value())
        self.assertEqual(tagged.as_mapping().keys(), ["a"])

    def test_tagged_sequence(self):
        tagged = Document.parse("t: !list\n  - 1\n")["t"].as_tagged()
        self.assertEqual(len(tagged.as_sequence()), 1)

    def test_non_tagged(self):
        node = Document.parse("a: 1\n")["a"]
        self.assertFalse(node.is_tagged())
        self.assertIsNone(node.as_tagged())

    def test_document_root_tagged(self):
        self.assertEqual(Document.parse("!foo bar\n").as_tagged().tag(), "!foo")
        self.assertIsNone(Document.parse("a: 1\n").as_tagged())

    def test_copy_tagged_value(self):
        doc = Document.parse("t: !custom val\n")
        doc.set("u", doc["t"].as_tagged())
        self.assertEqual(str(doc), "t: !custom val\nu: !custom val\n")


class PathTests(unittest.TestCase):
    YAML = (
        "server:\n"
        "  host: localhost\n"
        "  ports:\n"
        "    - 80\n"
        "    - 443\n"
        "name: x\n"
        "shared: &s\n"
        "  t: 1\n"
        "svc: *s\n"
    )

    def test_get_path(self):
        doc = Document.parse(self.YAML)
        self.assertEqual(str(doc.get_path("server.host")), "localhost")
        self.assertEqual(str(doc.get_path("server.ports[1]")), "443")
        self.assertEqual(str(doc.get_path("server.ports.0")), "80")

    def test_get_path_missing(self):
        doc = Document.parse(self.YAML)
        self.assertIsNone(doc.get_path("server.missing"))
        self.assertIsNone(Document().get_path("a"))

    def test_get_path_follows_alias(self):
        doc = Document.parse(self.YAML)
        self.assertEqual(str(doc.get_path("svc.t")), "1")
        with self.assertRaises(ValueError):
            doc.get_path("svc.t", alias_policy="refuse")

    def test_get_path_errors(self):
        doc = Document.parse(self.YAML)
        for path in ["", "server..host", "name.first"]:
            with self.subTest(path=path), self.assertRaises(ValueError):
                doc.get_path(path)

    def test_invalid_alias_policy(self):
        with self.assertRaises(ValueError):
            Document.parse(self.YAML).get_path("name", alias_policy="bogus")

    def test_set_path_creates_intermediates(self):
        doc = Document.parse("name: x\n")
        doc.set_path("db.primary.host", "h")
        self.assertEqual(str(doc), "name: x\ndb:\n  primary:\n    host: h\n")

    def test_set_path_sequence_index(self):
        doc = Document.parse("ports:\n  - 80\n")
        doc.set_path("ports[1]", 443)
        self.assertEqual(str(doc), "ports:\n  - 80\n  - 443\n")

    def test_set_path_through_scalar_raises(self):
        doc = Document.parse("name: x\n")
        with self.assertRaises(ValueError):
            doc.set_path("name.first", "y")
        self.assertEqual(str(doc), "name: x\n")

    def test_set_path_alias_policies(self):
        doc = Document.parse("shared: &s\n  t: 1\nsvc: *s\n")
        with self.assertRaises(ValueError):
            doc.set_path("svc.t", 2)
        doc.set_path("svc.t", 3, alias_policy="expand")
        self.assertEqual(str(doc), "shared: &s\n  t: 1\nsvc:\n  t: 3\n")

    def test_set_path_follow_alias(self):
        doc = Document.parse("shared: &s\n  t: 1\nsvc: *s\n")
        doc.set_path("svc.t", 2, alias_policy="follow")
        self.assertEqual(str(doc), "shared: &s\n  t: 2\nsvc: *s\n")

    def test_remove_path(self):
        doc = Document.parse(self.YAML)
        self.assertTrue(doc.remove_path("server.host"))
        self.assertFalse(doc.remove_path("server.host"))
        self.assertIsNone(doc.get_path("server.host"))
        self.assertFalse(Document().remove_path("a"))

    def test_remove_path_errors(self):
        with self.assertRaises(ValueError):
            Document.parse(self.YAML).remove_path("name.first")

    def test_mapping_paths(self):
        mapping = Document.parse(self.YAML).as_mapping()
        self.assertEqual(str(mapping.get_path("server.host")), "localhost")
        mapping.set_path("server.host", "example.com")
        self.assertEqual(str(mapping.get_path("server.host")), "example.com")
        self.assertTrue(mapping.remove_path("server.host"))


class OrderingTests(unittest.TestCase):
    def test_insert_after_and_before(self):
        doc = Document.parse("a: 1\nc: 3\n")
        self.assertTrue(doc.insert_after("a", "b", 2))
        self.assertTrue(doc.insert_before("a", "z", 0))
        self.assertEqual(str(doc), "z: 0\na: 1\nb: 2\nc: 3\n")

    def test_insert_after_missing_reference(self):
        doc = Document.parse("a: 1\n")
        self.assertFalse(doc.insert_after("nope", "b", 2))
        self.assertEqual(str(doc), "a: 1\n")

    def test_insert_existing_key_updates_in_place(self):
        doc = Document.parse("a: 1\nb: 2\nc: 3\n")
        self.assertTrue(doc.insert_after("c", "a", 10))
        self.assertEqual(str(doc), "a: 10\nb: 2\nc: 3\n")

    def test_move_after_and_before(self):
        doc = Document.parse("a: 1\nb: 2\nc: 3\n")
        self.assertTrue(doc.move_after("c", "a", 10))
        self.assertEqual(str(doc), "b: 2\nc: 3\na: 10\n")
        self.assertTrue(doc.move_before("b", "c", 30))
        self.assertEqual(str(doc), "c: 30\nb: 2\na: 10\n")
        self.assertFalse(doc.move_before("nope", "c", 1))

    def test_insert_at_index(self):
        doc = Document.parse("a: 1\nb: 2\n")
        doc.insert_at_index(1, "x", "y")
        doc.insert_at_index(100, "end", True)
        self.assertEqual(str(doc), "a: 1\nx: y\nb: 2\nend: true\n")

    def test_insert_at_index_empty_document(self):
        doc = Document()
        doc.insert_at_index(0, "k", "v")
        self.assertEqual(doc.keys(), ["k"])

    def test_reorder_fields(self):
        doc = Document.parse("a: 1\nb: 2\nc: 3\nd: 4\n")
        doc.reorder_fields(["c", "a"])
        self.assertEqual(str(doc), "c: 3\na: 1\nb: 2\nd: 4\n")

    def test_non_mapping_root_raises(self):
        doc = Document.parse("- 1\n")
        with self.assertRaises(TypeError):
            doc.insert_after("a", "b", 1)
        with self.assertRaises(TypeError):
            doc.move_before("a", "b", 1)
        with self.assertRaises(TypeError):
            doc.insert_at_index(0, "b", 1)
        with self.assertRaises(TypeError):
            doc.reorder_fields(["a"])
        self.assertEqual(str(doc), "- 1\n")

    def test_mapping_ordering(self):
        mapping = Document.parse("x: 1\ny: 2\n").as_mapping()
        self.assertTrue(mapping.insert_after("x", "n", Mapping(flow=True)))
        self.assertTrue(mapping.insert_before("x", "w", 0))
        self.assertTrue(mapping.move_after("y", "x", 1))
        mapping.insert_at_index(0, "first", "f")
        self.assertEqual(str(mapping), "first: f\nw: 0\nn: {}\ny: 2\nx: 1\n")
        mapping.reorder_fields(["y", "x"])
        self.assertEqual(mapping.keys(), ["y", "x", "first", "w", "n"])


class PositionTests(unittest.TestCase):
    SRC = "# top\nname: demo  # the name\nlist:\n  - a\n  - b\n"

    def test_node_positions(self):
        node = Document.parse(self.SRC)["name"]
        self.assertEqual(node.byte_range(), (12, 16))
        self.assertEqual(node.start_position(self.SRC), (2, 7))
        self.assertEqual(node.end_position(self.SRC), (2, 11))

    def test_sequence_and_scalar_positions(self):
        seq = Document.parse(self.SRC).as_mapping().get_sequence("list")
        start, end = seq.byte_range()
        self.assertEqual(self.SRC[start:end], "- a\n  - b\n")
        self.assertEqual(seq.start_position(self.SRC), (4, 3))
        self.assertEqual(seq[1].as_scalar().start_position(self.SRC), (5, 5))

    def test_entry_positions(self):
        entry = Document.parse(self.SRC).as_mapping().entries()[1]
        start, end = entry.byte_range()
        self.assertEqual(self.SRC[start:end], "list:\n  - a\n  - b\n")
        self.assertEqual(entry.start_position(self.SRC), (3, 1))

    def test_document_and_mapping_positions(self):
        doc = Document.parse(self.SRC)
        self.assertEqual(doc.byte_range(), (0, len(self.SRC)))
        self.assertEqual(doc.start_position(self.SRC), (1, 1))
        self.assertEqual(doc.as_mapping().start_position(self.SRC), (2, 1))

    def test_byte_offset_to_line_column(self):
        self.assertEqual(byte_offset_to_line_column("a\u00e9\nb", 3), (1, 3))
        self.assertEqual(byte_offset_to_line_column("a\u00e9\nb", 4), (2, 1))


class CommentTests(unittest.TestCase):
    SRC = "# top\nname: demo  # the name\n"

    def test_document_comments(self):
        comments = Document.parse(self.SRC).comments()
        self.assertTrue(all(isinstance(c, Comment) for c in comments))
        self.assertEqual([c.text() for c in comments], ["# top", "# the name"])
        self.assertEqual([c.content() for c in comments], ["top", "the name"])
        self.assertEqual(comments[1].start_position(self.SRC), (2, 13))

    def test_yaml_file_comments(self):
        comments = YamlFile.parse("# a\nx: 1\n---\n# b\ny: 2\n").comments()
        self.assertEqual([str(c) for c in comments], ["# a", "# b"])

    def test_set_content(self):
        doc = Document.parse(self.SRC)
        doc.comments()[1].set_content("renamed")
        self.assertEqual(str(doc), "# top\nname: demo  # renamed\n")

    def test_set_text(self):
        doc = Document.parse(self.SRC)
        doc.comments()[0].set_text("#header")
        self.assertEqual(str(doc), "#header\nname: demo  # the name\n")

    def test_handle_follows_edits(self):
        doc = Document.parse(self.SRC)
        comment = doc.comments()[1]
        comment.set_content("renamed")
        self.assertEqual(comment.text(), "# renamed")

    def test_set_text_non_printable(self):
        doc = Document.parse(self.SRC)
        with self.assertRaises(ValueError):
            doc.comments()[0].set_text("# x\x01y")
        self.assertEqual(str(doc), self.SRC)

    def test_set_text_invalid(self):
        doc = Document.parse(self.SRC)
        comment = doc.comments()[0]
        for text in ["no hash", "# two\nlines", "# cr\r"]:
            with self.subTest(text=text), self.assertRaises(ValueError):
                comment.set_text(text)
        with self.assertRaises(ValueError):
            comment.set_content("two\nlines")
        self.assertEqual(str(doc), self.SRC)


class RelaxedParseTests(unittest.TestCase):
    def test_document_parse_relaxed_errors(self):
        doc, errors = Document.parse_relaxed("a: [1, 2\nb: 3\n")
        self.assertIsInstance(doc, Document)
        self.assertEqual(len(errors), 1)
        self.assertIn("Unclosed flow sequence", errors[0])

    def test_document_parse_relaxed_clean(self):
        doc, errors = Document.parse_relaxed("a: 1\n")
        self.assertEqual(errors, [])
        self.assertEqual(doc.keys(), ["a"])

    def test_yaml_file_parse_relaxed(self):
        stream, errors = YamlFile.parse_relaxed("a: 1\n---\nb: [\n")
        self.assertEqual(len(stream), 2)
        self.assertEqual(len(errors), 1)


class YamlFileTests(unittest.TestCase):
    def test_multiple_documents(self):
        stream = YamlFile.parse("a: 1\n---\nb: 2\n")
        self.assertEqual(len(stream), 2)
        self.assertEqual([d.keys() for d in stream.documents()], [["a"], ["b"]])

    def test_roundtrip(self):
        text = "a: 1\n---\nb: 2\n"
        self.assertEqual(str(YamlFile.parse(text)), text)

    def test_first_document(self):
        stream = YamlFile.parse("a: 1\n---\nb: 2\n")
        self.assertEqual(stream.document().keys(), ["a"])


class TypedKeyTests(unittest.TestCase):
    def test_keys_round_trip(self):
        doc = Document.parse("1: one\ntrue: t\nname: x\n~: n\n")
        keys = doc.keys()
        self.assertEqual(keys, [1, True, "name", None])
        for key in keys:
            self.assertIn(key, doc)
        self.assertEqual(doc[1], "one")
        self.assertEqual(doc[True], "t")
        self.assertEqual(doc[None], "n")
        self.assertNotIn("1", doc)

    def test_set_typed_keys(self):
        mapping = Mapping()
        mapping[1] = "a"
        mapping[2.5] = "b"
        mapping["1"] = "c"
        self.assertEqual(str(mapping), "1: a\n2.5: b\n'1': c\n")
        self.assertEqual(mapping.keys(), [1, 2.5, "1"])

    def test_entry_key_matches_typed(self):
        entry = Document.parse("1: a\n").as_mapping().entries()[0]
        self.assertTrue(entry.key_matches(1))
        self.assertFalse(entry.key_matches("1"))

    def test_merged_typed_keys(self):
        merged = Document.parse("base: &b\n  1: x\nm:\n  <<: *b\n").merged()
        self.assertEqual(merged.get_merged("m").keys(), [1])


class DictProtocolTests(unittest.TestCase):
    def test_abc_registration(self):
        mapping = Document.parse("a: 1\n").as_mapping()
        self.assertIsInstance(mapping, collections.abc.MutableMapping)
        self.assertIsInstance(
            Document.parse("a: 1\n").merged(), collections.abc.Mapping
        )

    def test_get_default(self):
        doc = Document.parse("a: 1\n")
        self.assertEqual(doc.get("a"), 1)
        self.assertIsNone(doc.get("b"))
        self.assertEqual(doc.get("b", "dflt"), "dflt")
        self.assertEqual(doc.as_mapping().get("b", 0), 0)

    def test_pop(self):
        doc = Document.parse("a: 1\nb: 2\n")
        self.assertEqual(doc.pop("a"), 1)
        self.assertIsNone(doc.pop("a", None))
        self.assertEqual(doc.as_mapping().pop("a", 5), 5)
        with self.assertRaises(KeyError):
            doc.pop("a")
        with self.assertRaises(TypeError):
            doc.pop("a", 1, 2)
        self.assertEqual(str(doc), "b: 2\n")

    def test_popitem(self):
        mapping = Document.parse("a: 1\nb: 2\n").as_mapping()
        key, value = mapping.popitem()
        self.assertEqual((key, value), ("b", 2))
        self.assertEqual(mapping.keys(), ["a"])
        mapping.popitem()
        with self.assertRaises(KeyError):
            mapping.popitem()

    def test_setdefault(self):
        doc = Document.parse("a: 1\n")
        self.assertEqual(doc.setdefault("a", 5), 1)
        self.assertEqual(doc.setdefault("b", 5), 5)
        self.assertTrue(doc.setdefault("c").as_scalar().is_null())
        self.assertEqual(str(doc), "a: 1\nb: 5\nc: null\n")

    def test_update(self):
        doc = Document.parse("a: 1\n")
        doc.update({"a": 2, "b": 3}, c=4)
        doc.update([("d", 5)])
        doc.update(Document.parse("e: 6\n").as_mapping())
        self.assertEqual(str(doc), "a: 2\nb: 3\nc: 4\nd: 5\ne: 6\n")

    def test_iteration_and_len(self):
        doc = Document.parse("a: 1\nb: 2\n")
        self.assertEqual(list(doc), ["a", "b"])
        self.assertEqual(len(doc), 2)
        self.assertEqual(dict(doc.items()), {"a": 1, "b": 2})
        self.assertEqual(doc.values(), [1, 2])

    def test_clear_document(self):
        doc = Document.parse("a: 1\nb: 2\n")
        doc.clear()
        self.assertEqual(len(doc), 0)

    def test_field_order(self):
        doc = Document.parse("name: x\nversion: 1\n")
        doc.set("description", "d", field_order=["name", "description", "version"])
        self.assertEqual(doc.keys(), ["name", "description", "version"])
        mapping = Document.parse("b: 1\n").as_mapping()
        mapping.set("a", 0, field_order=["a", "b"])
        self.assertEqual(mapping.keys(), ["a", "b"])

    def test_get_mapping_and_sequence_on_document(self):
        doc = Document.parse("m:\n  a: 1\ns: [1]\n")
        self.assertEqual(doc.get_mapping("m").keys(), ["a"])
        self.assertEqual(len(doc.get_sequence("s")), 1)
        self.assertIsNone(doc.get_mapping("s"))


class ListProtocolTests(unittest.TestCase):
    def seq(self, text="- a\n- b\n- c\n- b\n"):
        return Document.parse(text).as_sequence()

    def test_abc_registration(self):
        self.assertIsInstance(Sequence(), collections.abc.MutableSequence)
        self.assertIsInstance(YamlFile(), collections.abc.Sequence)

    def test_negative_index(self):
        seq = self.seq()
        self.assertEqual(seq[-1], "b")
        self.assertEqual(seq.get(-2), "c")
        self.assertIsNone(seq.get(-10))
        with self.assertRaises(IndexError):
            seq[-10]
        seq[-1] = "z"
        self.assertEqual(seq.to_python(), ["a", "b", "c", "z"])
        self.assertTrue(seq.set(-4, "y"))
        self.assertFalse(seq.set(10, "y"))

    def test_slices(self):
        seq = self.seq()
        self.assertEqual(seq[1:3], ["b", "c"])
        self.assertEqual(seq[::-1], ["b", "c", "b", "a"])
        del seq[0:2]
        self.assertEqual(seq.to_python(), ["c", "b"])
        with self.assertRaises(TypeError):
            seq[0:1] = ["x"]

    def test_del_negative(self):
        seq = self.seq()
        del seq[-1]
        self.assertEqual(seq.to_python(), ["a", "b", "c"])
        with self.assertRaises(IndexError):
            del seq[5]

    def test_append_extend_insert(self):
        seq = self.seq("- a\n")
        seq.append("b")
        seq.extend(["c", 1])
        seq += [None]
        seq.insert(-1, "ins")
        seq.insert(100, "end")
        seq.insert(-100, "start")
        self.assertEqual(
            seq.to_python(), ["start", "a", "b", "c", 1, "ins", None, "end"]
        )

    def test_pop(self):
        seq = self.seq()
        self.assertEqual(seq.pop(), "b")
        self.assertEqual(seq.pop(0), "a")
        self.assertEqual(seq.pop(-1), "c")
        with self.assertRaises(IndexError):
            seq.pop(5)
        with self.assertRaises(IndexError):
            Sequence().pop()

    def test_remove_by_value(self):
        seq = self.seq()
        self.assertIsNone(seq.remove("b"))
        self.assertEqual(seq.to_python(), ["a", "c", "b"])
        with self.assertRaises(ValueError):
            seq.remove("zz")

    def test_remove_by_index_is_deprecated(self):
        seq = self.seq()
        with self.assertWarns(DeprecationWarning):
            removed = seq.remove(0)
        self.assertEqual(removed, "a")
        self.assertEqual(seq.to_python(), ["b", "c", "b"])

    def test_push_is_deprecated(self):
        seq = self.seq("- a\n")
        with self.assertWarns(DeprecationWarning):
            seq.push("b")
        self.assertEqual(seq.to_python(), ["a", "b"])

    def test_index_count_contains(self):
        seq = self.seq()
        self.assertEqual(seq.index("b"), 1)
        self.assertEqual(seq.index("b", 2), 3)
        with self.assertRaises(ValueError):
            seq.index("a", 1)
        self.assertEqual(seq.count("b"), 2)
        self.assertIn("c", seq)
        self.assertNotIn("z", seq)
        self.assertNotIn(object(), seq)

    def test_reverse(self):
        seq = self.seq("- a\n- {x: 1}\n- [1, 2]\n")
        seq.reverse()
        self.assertEqual(str(seq), "- [1, 2]\n- {x: 1}\n- a\n")
        self.assertEqual(list(reversed(seq)), ["a", {"x": 1}, [1, 2]])


class PythonValueTests(unittest.TestCase):
    def test_set_dict_and_list(self):
        doc = Document.parse("a: 1\n")
        doc["cfg"] = {"x": 1, "y": [1, {"z": None}], 2: True}
        self.assertEqual(
            str(doc),
            "a: 1\ncfg:\n  x: 1\n  y:\n  - 1\n  - z: null\n  2: true\n",
        )
        self.assertEqual(
            doc["cfg"].to_python(), {"x": 1, "y": [1, {"z": None}], 2: True}
        )

    def test_tuple_is_sequence(self):
        doc = Document()
        doc["t"] = (1, 2)
        self.assertEqual(doc["t"].to_python(), [1, 2])

    def test_big_int(self):
        doc = Document.parse("a: 1\n")
        doc["big"] = 2**80
        self.assertEqual(str(doc), "a: 1\nbig: 1208925819614629174706176\n")
        self.assertEqual(doc["big"].to_python(), 2**80)

    def test_self_referencing_value_rejected(self):
        value: list = []
        value.append(value)
        with self.assertRaises(ValueError):
            Document()["x"] = value

    def test_to_python(self):
        doc = Document.parse(
            "defaults: &d\n"
            "  t: 1\n"
            "prod:\n"
            "  <<: *d\n"
            "  h: x\n"
            "seq: [*d, 2]\n"
            "f: 1.5\n"
            "n: ~\n"
            's: "q\\n"\n'
            "y: yes\n"
            "set: !!set {a, b}\n"
            "omap: !!omap [{x: 1}, {y: 2}]\n"
            "pairs: !!pairs [{k: 1}, {k: 2}]\n"
            "bin: !!binary aGVsbG8=\n"
            "str: !!str 123\n"
            "custom: !thing {z: 1}\n"
        )
        self.assertEqual(
            doc.to_python(),
            {
                "defaults": {"t": 1},
                "prod": {"h": "x", "t": 1},
                "seq": [{"t": 1}, 2],
                "f": 1.5,
                "n": None,
                "s": "q\n",
                "y": "yes",
                "set": {"a", "b"},
                "omap": {"x": 1, "y": 2},
                "pairs": [("k", 1), ("k", 2)],
                "bin": b"hello",
                "str": "123",
                "custom": {"z": 1},
            },
        )

    def test_to_python_redefined_anchor(self):
        doc = Document.parse("- &x 1\n- *x\n- &x 2\n- *x\n")
        self.assertEqual(doc.to_python(), [1, 1, 2, 2])

    def test_to_python_tag_directives(self):
        stream = YamlFile.parse(
            "%TAG !y! tag:yaml.org,2002:\n---\ni: !y!int 3\ns: !y!set {a}\n"
        )
        self.assertEqual(stream[0].to_python(), {"i": 3, "s": {"a"}})

    def test_to_python_empty_document(self):
        self.assertIsNone(Document().to_python())

    def test_to_python_recursive_alias(self):
        doc = Document.parse("a: &a\n  b: *a\n")
        with self.assertRaises(ResolveError):
            doc.to_python()

    def test_to_python_invalid_binary(self):
        with self.assertRaises(YamlError):
            Document.parse('b: !!binary "@@@"\n').to_python()

    def test_merged_to_python(self):
        doc = Document.parse("d: &d\n  t: 1\np:\n  <<: *d\n  h: x\n")
        self.assertEqual(doc.merged().get_merged("p").to_python(), {"h": "x", "t": 1})
        self.assertEqual(doc["p"].as_mapping().merged().keys(), ["h", "t"])


class EqualityTests(unittest.TestCase):
    def test_node_equals_python_value(self):
        doc = Document.parse("port: 8080\nname: x\nlist: [1, 2]\nm: {a: 1}\n")
        self.assertEqual(doc["port"], 8080)
        self.assertNotEqual(doc["port"], "8080")
        self.assertEqual(doc["name"], "x")
        self.assertEqual(doc["list"], [1, 2])
        self.assertEqual(doc["m"], {"a": 1})
        self.assertNotEqual(doc["port"], object())

    def test_collections_equal(self):
        doc = Document.parse("a: 1\nb: [1, 2]\n")
        self.assertEqual(doc, {"b": [1, 2], "a": 1})
        self.assertEqual(doc.as_mapping(), {"a": 1, "b": [1, 2]})
        self.assertEqual(doc.get_sequence("b"), [1, 2])
        self.assertEqual(Document.parse("a: 1\n"), Document.parse("a:   1  \n"))

    def test_scalar_hash(self):
        doc = Document.parse("a: 8080\nb: x\n")
        self.assertEqual(hash(doc["a"]), hash(8080))
        self.assertEqual(hash(doc["b"].as_scalar()), hash("x"))

    def test_collections_unhashable(self):
        doc = Document.parse("a: {b: 1}\n")
        for value in [doc, doc.as_mapping(), Sequence(), doc["a"]]:
            with self.subTest(value=value), self.assertRaises(TypeError):
                hash(value)

    def test_alias_equality(self):
        doc = Document.parse("a: &x 1\nb: *x\n")
        self.assertEqual(Alias("x"), Alias("x"))
        self.assertNotEqual(Alias("x"), Alias("y"))
        self.assertEqual(doc["b"], Alias("x"))
        self.assertEqual(hash(Alias("x")), hash(Alias("x")))


class ScalarStyleTests(unittest.TestCase):
    def test_style_and_type(self):
        doc = Document.parse("a: \"q\"\nb: |\n  x\nc: 1.5\nd: 'y'\ne: >\n  f\n")
        cases = {
            "a": ("double-quoted", "string", "q"),
            "b": ("literal", "string", "x\n"),
            "c": ("plain", "float", 1.5),
            "d": ("single-quoted", "string", "y"),
            "e": ("folded", "string", "f\n"),
        }
        for key, (style, scalar_type, value) in cases.items():
            scalar = doc[key].as_scalar()
            with self.subTest(key=key):
                self.assertEqual(scalar.style(), style)
                self.assertEqual(scalar.scalar_type(), scalar_type)
                self.assertEqual(scalar.to_python(), value)

    def test_write_styled_values(self):
        doc = Document.parse("a: 1\n")
        doc["v"] = ScalarValue("1.0", style="double-quoted")
        doc["w"] = ScalarValue("x", tag="!foo")
        doc["l"] = ScalarValue("line1\nline2", style="literal")
        self.assertEqual(
            str(doc), 'a: 1\nv: "1.0"\nw: !foo x\nl: |-\n  line1\n  line2\n'
        )

    def test_scalar_value_attributes(self):
        value = ScalarValue("x", style="single-quoted", tag="!t")
        self.assertEqual(value.value(), "x")
        self.assertEqual(value.style(), "single-quoted")
        self.assertEqual(value.tag(), "!t")
        self.assertIsNone(ScalarValue("x").tag())

    def test_invalid_style(self):
        with self.assertRaises(ValueError):
            ScalarValue("x", style="bad")


class ErrorTests(unittest.TestCase):
    def test_parse_error(self):
        with self.assertRaises(ParseError) as cm:
            Document.parse("a: [\n")
        self.assertIsInstance(cm.exception, YamlError)
        self.assertIsInstance(cm.exception, ValueError)
        self.assertEqual((cm.exception.line, cm.exception.column), (2, 1))

    def test_path_error(self):
        with self.assertRaises(PathError):
            Document.parse("a: 1\n").get_path("a..b")

    def test_resolve_error(self):
        with self.assertRaises(ResolveError):
            Document.parse("<<: *nope\n").merged()

    def test_missing_file(self):
        with self.assertRaises(FileNotFoundError) as cm:
            Document.from_file("/nonexistent/file.yaml")
        self.assertEqual(cm.exception.filename, "/nonexistent/file.yaml")
        with self.assertRaises(FileNotFoundError):
            YamlFile().to_file("/nonexistent/file.yaml")


class FileTests(unittest.TestCase):
    def test_pathlike(self):
        import pathlib

        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "a.yaml"
            Document.parse("a: 1\n").to_file(path)
            self.assertEqual(Document.from_file(path).keys(), ["a"])
            YamlFile.parse("a: 1\n---\nb: 2\n").to_file(os.fspath(path))
            self.assertEqual(len(YamlFile.from_file(path)), 2)


class YamlFileSequenceTests(unittest.TestCase):
    def test_indexing_and_iteration(self):
        stream = YamlFile.parse("a: 1\n---\nb: 2\n")
        self.assertEqual(stream[0].keys(), ["a"])
        self.assertEqual(stream[-1].keys(), ["b"])
        self.assertEqual([d.keys() for d in stream], [["a"], ["b"]])
        with self.assertRaises(IndexError):
            stream[2]

    def test_append(self):
        stream = YamlFile.parse("a: 1\n")
        stream.append(Document.parse("b: 2\n"))
        self.assertEqual(len(stream), 2)
        with self.assertWarns(DeprecationWarning):
            stream.push_document(Document.parse("c: 3\n"))
        self.assertEqual(len(stream), 3)

    def test_directives(self):
        stream = YamlFile.parse("%YAML 1.2\n---\na: 1\n")
        directives = stream.directives()
        self.assertEqual(directives, ["%YAML 1.2"])
        self.assertIsInstance(directives[0], Directive)
        self.assertEqual(directives[0].name(), "YAML")
        self.assertEqual(directives[0].value(), "1.2")
        self.assertTrue(directives[0].is_yaml_version())
        stream.add_directive(Directive.tag("!e!", "tag:example.com,2000:"))
        self.assertIn("%TAG !e! tag:example.com,2000:", stream.directives())
        self.assertTrue(Directive.tag("!e!", "x").is_tag())
        self.assertEqual(str(Directive.yaml_version("1.2")), "%YAML 1.2")

    def test_copy(self):
        stream = YamlFile.parse("a: 1\n")
        clone = copy.copy(stream)
        clone[0]["a"] = 2
        self.assertEqual(str(stream), "a: 1\n")
        self.assertEqual(str(clone), "a: 2\n")


class DocumentExtrasTests(unittest.TestCase):
    def test_root(self):
        self.assertIsNone(Document().root())
        self.assertEqual(Document.parse("- 1\n").root().kind, "sequence")

    def test_anchors(self):
        doc = Document.parse("a: &x 1\nb: &y [1]\n")
        self.assertEqual(sorted(doc.anchors()), ["x", "y"])

    def test_copy(self):
        doc = Document.parse("a: 1  # c\n")
        clone = copy.deepcopy(doc)
        clone["a"] = 2
        self.assertEqual(str(doc), "a: 1  # c\n")
        self.assertEqual(str(clone), "a: 2  # c\n")

    def test_validate(self):
        violations = Document.parse("a: 1\na: 2\n").validate()
        self.assertEqual(len(violations), 1)
        violation = violations[0]
        self.assertEqual(violation.rule, "duplicate_keys")
        self.assertEqual(violation.severity, "error")
        self.assertEqual(violation.location, "2:1")
        self.assertEqual(violation.byte_range, (5, 10))
        self.assertEqual(
            Document.parse("a: 1\na: 2\n").validate(duplicate_keys=False), []
        )
        self.assertEqual(YamlFile.parse("a: 1\n").validate(), [])

    def test_validate_schema(self):
        doc = Document.parse("a: 1\nb: x\n")
        self.assertEqual(doc.validate_schema(), [])
        self.assertEqual(doc.validate_schema("failsafe"), [])
        violations = doc.validate_schema("failsafe", strict=True)
        self.assertEqual([v.path for v in violations], ["root.a"])
        self.assertEqual(violations[0].schema, "failsafe")
        with self.assertRaises(ValueError):
            doc.validate_schema("bogus")

    def test_migrate(self):
        doc = Document.parse("mode: 0755\nflag: yes\n")
        self.assertEqual(
            doc.migrate_to_yaml_1_2(), [("0755", "0o755"), ("yes", "true")]
        )
        self.assertEqual(str(doc), "mode: 0o755\nflag: true\n")


class TaggedCollectionTests(unittest.TestCase):
    def test_as_set(self):
        tagged = Document.parse("s: !!set {a, b}\n")["s"].as_tagged()
        self.assertEqual(tagged.as_set(), ["a", "b"])
        self.assertEqual(tagged.to_python(), {"a", "b"})

    def test_as_ordered_mapping_and_pairs(self):
        doc = Document.parse("o: !!omap [{x: 1}]\np: !!pairs [{k: 1}, {k: 2}]\n")
        self.assertEqual(doc["o"].as_tagged().as_ordered_mapping(), [("x", 1)])
        self.assertEqual(doc["p"].as_tagged().as_pairs(), [("k", 1), ("k", 2)])
        self.assertIsNone(doc["o"].as_tagged().as_set())


if __name__ == "__main__":
    unittest.main()
