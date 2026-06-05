import unittest

from yaml_edit import Document, Entry, Mapping, Scalar, Sequence, YamlFile


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

    def test_none_rejected(self):
        doc = Document()
        with self.assertRaises(TypeError):
            doc.set("x", None)

    def test_unsupported_type_rejected(self):
        doc = Document()
        with self.assertRaises(TypeError):
            doc.set("x", [1, 2, 3])


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
