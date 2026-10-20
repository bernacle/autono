import contextlib
import io
import unittest
import cli


class BaselineTests(unittest.TestCase):
    def test_hello(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(cli.main(["hello"]), 0)
        self.assertEqual(out.getvalue(), "hello\n")

    def test_missing_command(self):
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            self.assertEqual(cli.main([]), 2)
        self.assertIn("usage:", err.getvalue())

    def test_unknown_command(self):
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(cli.main(["unknown"]), 2)


if __name__ == "__main__":
    unittest.main()
