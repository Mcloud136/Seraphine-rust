import os
import unittest
from unittest.mock import patch

from app.common.util import (
    _enumerate_processes,
    _parse_lcu_command_line,
    _read_process_command_line,
    getPortTokenServerByPid,
    getTasklistPath,
)


class WindowsApiProcessTests(unittest.TestCase):
    def test_toolhelp_enumerates_current_process(self):
        self.assertEqual(getTasklistPath(), "windows-api")
        processes = dict(_enumerate_processes())
        self.assertIn(os.getpid(), processes)
        self.assertTrue(processes[os.getpid()].lower().endswith(".exe"))

    def test_peb_reads_current_process_command_line(self):
        command_line = _read_process_command_line(os.getpid())
        self.assertIsInstance(command_line, str)
        self.assertTrue(command_line)

    def test_lcu_arguments_are_parsed_without_shell(self):
        for server in ("HN1", "HN10", "EUW1", "NA1", "BG2"):
            with self.subTest(server=server):
                command_line = (
                    '"LeagueClientUx.exe" --app-port=55123 '
                    f'--remoting-auth-token=test-token --rso_platform_id={server}'
                )
                self.assertEqual(
                    _parse_lcu_command_line(command_line),
                    ("55123", "test-token", server),
                )

    def test_psutil_failure_falls_back_to_peb(self):
        command_line = _read_process_command_line(os.getpid())
        expected = _parse_lcu_command_line(command_line)
        with patch(
            "app.common.util.getPortTokenServerByPidViaPsutil",
            side_effect=OSError("access denied"),
        ):
            self.assertEqual(getPortTokenServerByPid(os.getpid()), expected)


if __name__ == "__main__":
    unittest.main()
