#!/usr/bin/env python3
"""Run a required trusted Python suite with a completed successful test outcome.

The checkout verifier supplies the suite path and bounds this process. Discovery
imports trusted repository test code; this wrapper is not candidate isolation.
"""
from pathlib import Path
import argparse
import sys
import unittest


class RequiredResult(unittest.TextTestResult):
    """Track actual successes; skipped subtests are not skipped test methods."""

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.successful_outcomes = 0

    def addSuccess(self, test):
        super().addSuccess(test)
        self.successful_outcomes += 1

    def addSubTest(self, test, subtest, error):
        super().addSubTest(test, subtest, error)
        if error is None:
            self.successful_outcomes += 1


def run_suite(directory, pattern, *, stream=sys.stderr):
    suite = unittest.TestLoader().discover(str(directory), pattern=pattern)
    discovered = suite.countTestCases()
    if not discovered:
        print('Required suite discovered no tests.', file=stream)
        return 1
    print(f'Required suite discovered {discovered} tests.', file=stream)
    result = unittest.TextTestRunner(stream=stream, verbosity=2, resultclass=RequiredResult).run(suite)
    if not result.successful_outcomes:
        print('Required suite completed no successful test or subtest.', file=stream)
        return 1
    return 0 if result.wasSuccessful() else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('pattern')
    args = parser.parse_args()
    return run_suite(args.directory, args.pattern)


if __name__ == '__main__':
    raise SystemExit(main())
