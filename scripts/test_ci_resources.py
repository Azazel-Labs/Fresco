import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "ci_resources", Path(__file__).with_name("ci-resources.py")
)
resources = importlib.util.module_from_spec(spec)
spec.loader.exec_module(resources)


class ResourceTests(unittest.TestCase):
    def test_cpu_and_memory_limits(self):
        for cpus, gib, expected in [
            (4, 16, 4),
            (8, 16, 4),
            (4, 7, 2),
            (2, 64, 2),
            (1, 1, 2),
            (8, 0, 2),
            (8, 8, 2),
        ]:
            with self.subTest(cpus=cpus, gib=gib):
                self.assertEqual(
                    resources.build_jobs(cpus, gib * resources.GIB), expected
                )

    def test_memory_boundary(self):
        self.assertEqual(resources.build_jobs(8, 11 * resources.GIB - 1), 2)
        self.assertEqual(resources.build_jobs(8, 11 * resources.GIB), 3)

    def test_uses_available_not_total_memory(self):
        self.assertEqual(
            resources.available_memory("MemTotal: 16000000 kB\nMemAvailable: 5000000 kB\n"),
            5000000 * 1024,
        )

    def test_invalid_resource_data_fails(self):
        for cpus, memory in [(0, 1024), (4, -1)]:
            with self.assertRaises(ValueError):
                resources.build_jobs(cpus, memory)
        for meminfo in ["", "MemAvailable: 123 bytes", "MemAvailable: invalid kB"]:
            with self.assertRaises(ValueError):
                resources.available_memory(meminfo)


if __name__ == "__main__":
    unittest.main()
