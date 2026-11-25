from PyQt5.QtCore import Qt, QRect, QSize
from PyQt5.QtGui import QPainter, QImage, QBrush, QColor

from ..backend.image_pool import AsyncImageMixin
from ..common.qfluentwidgets import NavigationWidget, isDarkTheme


class NavigationAvatarWidget(AsyncImageMixin, NavigationWidget):
    """Avatar widget"""

    def __init__(self, avatar: str, name: str, parent=None):
        super().__init__(isSelectable=False, parent=parent)

        self.name = name
        self._avatar = QImage()
        self._initAsyncImage("navigation-avatar", target_size=QSize(24, 24))
        self.setAvatar(avatar)

    def setAvatar(self, path):
        self.setAsyncImage(path)

    def _acceptAsyncImage(self, image):
        self._avatar = image
        self.update()

    def _clearAsyncImage(self):
        self._avatar = QImage()
        self.update()

    def paintEvent(self, e):
        painter = QPainter(self)
        painter.setRenderHints(
            QPainter.SmoothPixmapTransform | QPainter.Antialiasing)

        painter.setPen(Qt.NoPen)

        if self.isPressed:
            painter.setOpacity(0.7)

        # draw background
        if self.isEnter:
            c = 255 if isDarkTheme() else 0
            painter.setBrush(QColor(c, c, c, 10))
            painter.drawRoundedRect(self.rect(), 5, 5)

        # draw avatar
        painter.setBrush(QBrush(self._avatar))
        painter.translate(8, 6)
        painter.drawEllipse(0, 0, 24, 24)
        painter.translate(-8, -6)

        if not self.isCompacted:
            painter.setPen(Qt.white if isDarkTheme() else Qt.black)

            painter.drawText(QRect(44, -1, 255, 36),
                             Qt.AlignVCenter, self.name)
            self.setStyleSheet(
                "NavigationWidget{font: 14px 'Segoe UI', 'Microsoft YaHei'}"
            )
