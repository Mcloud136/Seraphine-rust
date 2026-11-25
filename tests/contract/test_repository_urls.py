import unittest
from unittest.mock import Mock, patch

from app.common.config import FEEDBACK_URL, GITHUB_URL
from app.common.util import Github


class RepositoryUrlTests(unittest.TestCase):
    def test_public_links_point_to_maintained_repository(self):
        self.assertEqual(GITHUB_URL, "https://github.com/Mcloud136/Seraphine-rust")
        self.assertEqual(
            FEEDBACK_URL,
            "https://github.com/Mcloud136/Seraphine-rust/issues?q=is%3Aissue",
        )

    def test_update_check_uses_maintained_repository_over_https(self):
        github = Github()
        github.sess = Mock()
        github.sess.get.return_value.json.return_value = {}
        with patch("app.common.util.cfg.get", return_value=False):
            github.getReleasesInfo()
        github.sess.get.assert_called_once_with(
            "https://api.github.com/repos/Mcloud136/Seraphine-rust/releases/latest",
            proxies=None,
        )


if __name__ == "__main__":
    unittest.main()
