import unittest

from yaml_edit import (
    Alias,
    Document,
    Entry,
    Mapping,
    MergedMapping,
    Scalar,
    Sequence,
    TaggedNode,
    YamlFile,
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
            doc.set("x", [1, 2, 3])

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
        seq.push("c")
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
        seq.push("b")
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
        seq.push(None)
        seq.push(Sequence(flow=True))
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


if __name__ == "__main__":
    unittest.main()
