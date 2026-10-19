# Disposable acceptance sample

This tiny standard-library Python CLI is a seed for Autono's opt-in real-agent
acceptance test. It has working tests and intentionally does not yet validate
configuration. The test copies these files into a new local Git repository;
agents never edit this source fixture or the Autono checkout.

Run the baseline with `python3 -m unittest discover -v`.
