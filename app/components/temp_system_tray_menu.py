from PyQt5.QtGui import QCursor
from PyQt5.QtWidgets import QApplication
from ..common.qfluentwidgets import (SystemTrayMenu)


class TmpSystemTrayMenu(SystemTrayMenu):
    def __init__(self, parent=None):
        super().__init__(parent=parent)

    def adjustPosition(self):
        m = self.layout().contentsMargins()
        rect = QApplication.screenAt(QCursor.pos()).availableGeometry()

        w, h = self.layout().sizeHint().width() + 5, self.layout().sizeHint().height()

        x = min(self.x() - m.left(), rect.right() - w)
        y = QCursor.pos().y() - self.height() + m.bottom()

        self.move(x, y)
