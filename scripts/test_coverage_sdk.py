"""Regression coverage for the environment passed to the instrumented build."""
import shlex
import unittest

from scripts.coverage_sdk import parse_show_env


class ShowEnv(unittest.TestCase):
    def test_flags_reach_rustc_without_literal_shell_quotes(self):
        flags = '-D warnings -C instrument-coverage --cfg=coverage'
        env = parse_show_env(f'export RUSTFLAGS={shlex.quote(flags)}\n')
        self.assertEqual(env['RUSTFLAGS'].split(),
                         ['-D', 'warnings', '-C', 'instrument-coverage', '--cfg=coverage'])

    def test_encoded_flags_paths_and_shell_metacharacters_are_preserved(self):
        expected = {
            'CARGO_ENCODED_RUSTFLAGS': '-D\x1fwarnings\x1f-C\x1finstrument-coverage',
            'RUSTDOCFLAGS': '-C instrument-coverage --cfg=coverage',
            'LLVM_PROFILE_FILE': r"C:\work space\O'Brien\profiles-%p-%m.profraw",
            'EMPTY': '',
            'LITERAL': '$(do-not-execute)=value',
        }
        settings = '\r\n'.join(f'export {k}={shlex.quote(v)}' for k, v in expected.items())
        self.assertEqual(parse_show_env(settings), expected)

    def test_unexpected_output_is_rejected(self):
        for line in ('RUSTFLAGS=bad', 'export RUSTFLAGS=one two', 'export NO_EQUALS'):
            with self.subTest(line=line), self.assertRaises(ValueError):
                parse_show_env(line)
