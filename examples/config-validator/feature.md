# Validate a configuration file

Implement one cohesive CLI feature in this small Python repository:
`python3 cli.py validate PATH` reads a UTF-8 JSON configuration file.

Acceptance criteria:

- Preserve the existing `hello` command and its tests.
- A valid configuration is a JSON object with a nonempty string `name` and an
  integer `port` between 1 and 65535 inclusive. Reject booleans as port values.
- Valid input prints a clear success message to stdout and exits 0.
- Missing/unreadable files, invalid UTF-8, malformed JSON, a non-object root,
  missing/invalid fields, and wrong argument counts produce meaningful errors
  on stderr and a nonzero exit, without a Python traceback.
- Add deterministic standard-library unit tests covering valid input, boundaries,
  and the listed errors. No third-party dependencies or network access.

Keep this as a single implementation task unless there is a concrete dependency
that requires more tasks. Verify using `python3 -m unittest discover -v`.
