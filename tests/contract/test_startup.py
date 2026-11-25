import unittest
from types import SimpleNamespace

from PyQt5.QtGui import QFont
from PyQt5.QtWidgets import QApplication, QLabel, QWidget

from app.components.temp_system_tray_menu import TmpSystemTrayMenu
from app.view.main_window import MainWindow


class StartupTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def test_body_labels_inherit_fluent_font(self):
        reference = QLabel()
        reference.setFont(QFont("Segoe UI", 9))
        body = QLabel("body")
        body.setFont(QFont(""))
        fake_window = SimpleNamespace(
            settingInterface=SimpleNamespace(
                apiConcurrencyCount=SimpleNamespace(
                    card=SimpleNamespace(titleLabel=reference)
                )
            )
        )
        MainWindow._MainWindow__normalizeLabelFonts(fake_window)
        self.assertEqual(body.font().family(), "Segoe UI")
        body.deleteLater()
        reference.deleteLater()
        self.app.processEvents()

    def test_tray_menu_keeps_window_as_parent(self):
        parent = QWidget()
        menu = TmpSystemTrayMenu(parent=parent)
        try:
            self.assertEqual(menu.title(), "")
            self.assertIs(menu.parent(), parent)
        finally:
            menu.deleteLater()
            parent.deleteLater()
            self.app.processEvents()


if __name__ == "__main__":
    unittest.main()
