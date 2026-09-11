import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("python-api-surface.py")
SPEC = importlib.util.spec_from_file_location("python_api_surface", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
api_surface = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(api_surface)


class PythonApiSurfaceTests(unittest.TestCase):
    def manifest(self, source: str) -> dict:
        with tempfile.NamedTemporaryFile("w", suffix=".pyi", delete=False) as stub:
            stub.write(source)
            path = stub.name
        try:
            return api_surface.build_manifest(path)
        finally:
            Path(path).unlink()

    def test_annotated_attribute_and_property_method_normalize_to_property(self) -> None:
        annotated = api_surface.symbol_map(self.manifest("class Entry:\n    port: int\n"))
        decorated = api_surface.symbol_map(
            self.manifest(
                "class Entry:\n"
                "    @property\n"
                "    def port(self) -> int: ...\n"
            )
        )
        self.assertIn("property:Entry.port", annotated)
        self.assertIn("property:Entry.port", decorated)
        self.assertNotIn("method:Entry.port", decorated)

    def test_safety_removals_have_reviewed_dispositions(self) -> None:
        self.assertEqual(
            api_surface.RECORDED_REMOVALS[
                "method:BvllPolicyVerdict.allow_and_forward"
            ]["disposition"],
            "intentional-removal",
        )
        self.assertEqual(
            api_surface.RECORDED_REMOVALS["method:BACnetClient.who_is_router"][
                "disposition"
            ],
            "superseded",
        )


if __name__ == "__main__":
    unittest.main()
