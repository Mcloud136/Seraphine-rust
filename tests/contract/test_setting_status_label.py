import unittest

from PyQt5.QtWidgets import QApplication

from app.components.setting_cards import HeaderStatusLabel


class HeaderStatusLabelTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def test_minimum_width_tracks_status_text(self):
        label = HeaderStatusLabel()
        for text in ("Now: 20", "Enabled, delay: 10 seconds", "Disabled"):
            with self.subTest(text=text):
                label.setText(text)
                self.app.processEvents()
                self.assertGreaterEqual(label.minimumWidth(), label.sizeHint().width())
                self.assertTrue(label.isHidden())
        label.deleteLater()
        self.app.processEvents()


if __name__ == "__main__":
    unittest.main()
