import unittest
from types import SimpleNamespace
from unittest.mock import patch

from PyQt5.QtCore import QLocale

from app.backend.catalog import CatalogManager
from app.common.config import Language, get_effective_language


class LanguageTests(unittest.TestCase):
    def test_auto_uses_system_language(self):
        cases = [
            (QLocale(QLocale.Chinese, QLocale.China), Language.CHINESE_SIMPLIFIED),
            (QLocale(QLocale.Portuguese, QLocale.Brazil), Language.PORTUGUESE),
            (QLocale(QLocale.English, QLocale.UnitedStates), Language.ENGLISH),
        ]
        for locale, expected in cases:
            with self.subTest(locale=locale.name()), patch(
                "app.common.config.cfg",
                SimpleNamespace(language=SimpleNamespace(value=Language.AUTO)),
            ), patch("app.common.config.QLocale.system", return_value=locale):
                self.assertEqual(get_effective_language(), expected)

    def test_auto_chinese_translates_map_names(self):
        with patch(
            "app.common.config.cfg",
            SimpleNamespace(language=SimpleNamespace(value=Language.AUTO)),
        ), patch(
            "app.common.config.QLocale.system",
            return_value=QLocale(QLocale.Chinese, QLocale.China),
        ):
            self.assertEqual(CatalogManager().getMapNameById(12), "嚎哭深渊")


if __name__ == "__main__":
    unittest.main()
