"""A deliberately small CLI used as an engineering-loop acceptance seed."""
import sys


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if argv == ["hello"]:
        print("hello")
        return 0
    print("usage: cli.py hello", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
