from PyQt5.QtCore import Qt, QRect, pyqtSignal, pyqtSlot
from PyQt5.QtGui import QPainter, QImage, QBrush, QColor

from ..backend.image_pool import get_image_pool
from ..common.qfluentwidgets import NavigationWidget, isDarkTheme


class NavigationAvatarWidget(NavigationWidget):
    """Avatar widget"""

    def __init__(self, avatar: str, name: str, parent=None):
        super().__init__(isSelectable=False, parent=parent)

        self.name = name
        self._avatar = QImage()
        self._avatarKey = None
        self._imagePool = get_image_pool()
        self._imagePool.imageReady.connect(self._onImageReady)
        self.setAvatar(avatar)

    def setAvatar(self, path):
        dpr = self.devicePixelRatioF()
        self._avatarKey = f"navigation-avatar:{path}:{dpr:.2f}"
        self._imagePool.submit(self._avatarKey, path, 24, 24, dpr)

    @pyqtSlot(str, QImage)
    def _onImageReady(self, key, image):
        if key != self._avatarKey:
            return
        self._avatar = image
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
