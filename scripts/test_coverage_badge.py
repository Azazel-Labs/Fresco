import unittest
import xml.etree.ElementTree as ET

from coverage_badge import render_badge


def summary(count, covered):
    return {"data": [{"totals": {"lines": {"count": count, "covered": covered}}}]}


class CoverageBadgeTests(unittest.TestCase):
    def test_measured_ratio_and_rounding(self):
        badge = ET.fromstring(render_badge(summary(300, 167)))
        self.assertEqual(badge.attrib["aria-label"], "Rust lines: 55.7%")
        self.assertIn("167 of 300 lines", "".join(badge.itertext()))

    def test_zero_and_complete_coverage(self):
        for covered, value in [(0, "0.0%"), (10, "100.0%")]:
            with self.subTest(covered=covered):
                self.assertIn(f"Rust lines: {value}", render_badge(summary(10, covered)))

    def test_invalid_counts_do_not_publish_a_percentage(self):
        for count, covered in [(0, 0), (10, -1), (10, 11), (10, 1.5), (True, 1)]:
            with self.subTest(count=count, covered=covered):
                with self.assertRaises(ValueError):
                    render_badge(summary(count, covered))

    def test_missing_or_ambiguous_summary_is_rejected(self):
        with self.assertRaises(KeyError):
            render_badge({})
        for data in [[], summary(10, 5)["data"] * 2]:
            with self.assertRaises(ValueError):
                render_badge({"data": data})


if __name__ == "__main__":
    unittest.main()
