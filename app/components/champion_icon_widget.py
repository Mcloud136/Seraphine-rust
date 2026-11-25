from PyQt5.QtCore import QEvent, Qt, pyqtSignal, QRectF, QSize
from PyQt5.QtGui import (QColor, QMouseEvent, QPainter, QPainterPath, QLinearGradient, QGradient,
                         QPen, QPixmap, qGray, qAlpha, qRgba)
from PyQt5.QtWidgets import QWidget, QFrame, QLabel, QGraphicsOpacityEffect

from app.backend.image_pool import AsyncImageMixin
from app.common.qfluentwidgets import isDarkTheme

import time


class RoundIcon(AsyncImageMixin, QFrame):
    def __init__(self, icon=None, diameter=None, overscaled=0,
                 borderWidth=1, drawBackground=False, enabled=True, parent=None) -> None:
        super().__init__(parent)
        self.image = QPixmap()
        self._initAsyncImage("round-icon", QSize(diameter or 64, diameter or 64))

        self.overscaled = overscaled
        self.borderWidth = borderWidth
        self.drawBackground = drawBackground
        self.enabled = enabled
        self.havePic = False

        self.setFixedSize(diameter, diameter)
        if icon is not None:
            self.setIcon(icon)

    def _acceptAsyncImage(self, image):
        self.image = QPixmap.fromImage(image)
        self.havePic = True
        self.update()

    def _clearAsyncImage(self):
        self.image = QPixmap()
        self.havePic = False
        self.update()

    def paintEvent(self, event) -> None:
        if not self.havePic:
            return

        painter = QPainter(self)
        painter.setRenderHint(QPainter.Antialiasing)

        width = self.image.width() - 2*self.overscaled
        height = self.image.height() - 2*self.overscaled

        image = self.image.copy(
            self.overscaled, self.overscaled, width, height)

        size = self.size() * self.devicePixelRatioF()
        image: QPixmap = image.scaled(size,
                                      Qt.AspectRatioMode.KeepAspectRatio,
                                      Qt.TransformationMode.SmoothTransformation)

        path = QPainterPath()
        path.addEllipse(0, 0, self.width(), self.height())

        painter.setClipPath(path)

        if not self.enabled:
            painter.setOpacity(0.15)

        if self.drawBackground:
            painter.save()
            painter.setBrush(QColor(0, 0, 0))
            painter.drawEllipse(0, 0, self.width(), self.height())
            painter.restore()

        painter.drawPixmap(self.rect(), image)

        if self.borderWidth != 0 and self.enabled:
            painter.save()
            painter.setPen(
                QPen(QColor(120, 90, 40), self.borderWidth, Qt.SolidLine))
            painter.drawEllipse(0, 0, self.width(), self.height())
            painter.restore()

        return super().paintEvent(event)

    def setIcon(self, icon):
        self.setAsyncImage(icon)

    def setEnabeld(self, enabled):
        self.enabled = enabled

        self.repaint()


class RoundIconButton(AsyncImageMixin, QFrame):
    clicked = pyqtSignal(int)

    def __init__(self, icon, diameter, overscaled, borderWidth, championName, championId, parent=None) -> None:
        super().__init__(parent)

        self.image = QPixmap()
        self._initAsyncImage("round-icon-button", QSize(diameter, diameter))

        self.borderWidth = borderWidth
        self.overscaled = overscaled

        self.championName: str = championName
        self.championId = championId

        self.isPressed = False
        self.isHover = False

        self.setFixedSize(diameter, diameter)
        self.setAsyncImage(icon)

    def _acceptAsyncImage(self, image):
        self.image = QPixmap.fromImage(image)
        self.update()

    def _clearAsyncImage(self):
        self.image = QPixmap()
        self.update()

    def paintEvent(self, event) -> None:
        painter = QPainter(self)
        painter.setRenderHint(QPainter.Antialiasing)

        path = QPainterPath()
        path.addEllipse(0, 0, self.width(), self.height())
        painter.setClipPath(path)

        width = self.image.width() - 2*self.overscaled
        height = self.image.height() - 2*self.overscaled

        image = self.image.copy(
            self.overscaled, self.overscaled, width, height)

        size = self.size() * self.devicePixelRatioF()
        image = image.scaled(size,
                             Qt.AspectRatioMode.KeepAspectRatio,
                             Qt.TransformationMode.SmoothTransformation)

        if self.isPressed:
            painter.setOpacity(0.63)
        elif self.isHover:
            painter.setOpacity(0.80)
        else:
            painter.setOpacity(1)

        painter.drawPixmap(self.rect(), image)

        painter.setPen(
            QPen(QColor(120, 90, 40), self.borderWidth, Qt.SolidLine))
        painter.drawEllipse(0, 0, self.width(), self.height())

        return super().paintEvent(event)

    def enterEvent(self, a0: QEvent) -> None:
        self.isHover = True
        self.update()
        return super().enterEvent(a0)

    def leaveEvent(self, a0: QEvent) -> None:
        self.isHover = False
        self.update()
        return super().leaveEvent(a0)

    def mousePressEvent(self, a0: QMouseEvent) -> None:
        self.isPressed = True
        self.update()
        return super().mousePressEvent(a0)

    def mouseReleaseEvent(self, a0: QMouseEvent) -> None:
        self.isPressed = False
        self.update()
        ret = super().mouseReleaseEvent(a0)
        self.clicked.emit(self.championId)
        return ret


class AsyncPixmapLabel(AsyncImageMixin, QLabel):
    def __init__(self, imagePath=None, target_size=None, parent=None):
        super().__init__(parent)
        self._initAsyncImage("pixmap-label", target_size=target_size)
        if imagePath is not None:
            self.setPicture(imagePath)

    def _acceptAsyncImage(self, image):
        self.setPixmap(QPixmap.fromImage(image))

    def _clearAsyncImage(self):
        self.setPixmap(QPixmap())

    def setPicture(self, imagePath):
        if isinstance(imagePath, QPixmap):
            self.cancelAsyncImage()
            self.setPixmap(imagePath)
            return
        self.setAsyncImage(imagePath)

    def setText(self, text):
        self.cancelAsyncImage()
        return super().setText(text)

    def clear(self):
        self.cancelAsyncImage()
        return super().clear()


class TopRoundedLabel(AsyncImageMixin, QLabel):
    def __init__(self, imagePath=None, radius=4.0, parent=None):
        super().__init__(parent)
        self._initAsyncImage("top-rounded", use_widget_size=True)

        self.havePic = False
        self.radius = radius

        self.opacity = QGraphicsOpacityEffect(opacity=1)
        self.setGraphicsEffect(self.opacity)
        if imagePath is not None:
            self.setPicture(imagePath)

    def paintEvent(self, e):
        if not self.havePic:
            return super().paintEvent(e)

        painter = QPainter(self)
        painter.setRenderHints(QPainter.Antialiasing)

        pixmap = self.pixmap().scaled(
            self.size()*self.devicePixelRatioF(),
            Qt.IgnoreAspectRatio, Qt.SmoothTransformation)

        path = QPainterPath()

        topPath = QPainterPath()
        topRect = QRectF(self.rect().x(), self.rect().y(),
                         self.rect().width(), self.rect().height())
        topPath.addRoundedRect(topRect, self.radius, self.radius)

        bottomPath = QPainterPath()
        bottomRect = QRectF(self.rect().x(), self.rect().y() + self.rect().height() / 2,
                            self.rect().width(), self.rect().height() / 2)
        bottomPath.addRect(bottomRect)

        path = topPath.united(bottomPath)
        painter.setClipPath(path)

        grad = QLinearGradient(0, 0, 0, self.rect().height())
        grad.setColorAt(0.7, Qt.GlobalColor.black)
        grad.setColorAt(1, Qt.GlobalColor.transparent)
        self.opacity.setOpacityMask(grad)

        painter.drawPixmap(self.rect(), pixmap)

    def _acceptAsyncImage(self, image):
        self.setPixmap(QPixmap.fromImage(image))
        self.havePic = True
        self.repaint()

    def _clearAsyncImage(self):
        self.setPixmap(QPixmap())
        self.havePic = False
        self.repaint()

    def setPicture(self, imagePath):
        if isinstance(imagePath, QPixmap):
            self.cancelAsyncImage()
            self.setPixmap(imagePath)
            self.havePic = not imagePath.isNull()
            self.repaint()
            return
        self.setAsyncImage(imagePath)

    def setRedius(self, radius):
        self.radius = radius
        self.repaint()

    def setText(self, text):
        self.cancelAsyncImage()
        return super().setText(text)

    def clear(self):
        self.cancelAsyncImage()
        return super().clear()


class RoundedLabel(AsyncImageMixin, QLabel):
    def __init__(self, imagePath=None, radius=4.0, borderWidth=2, borderColor: QColor = None, drawBackground=False, parent=None):
        super().__init__(parent)
        self._initAsyncImage("rounded-label")

        self.havePic = False
        self.radius = radius
        self.borderWidth = borderWidth
        self.borderColor = borderColor if borderColor else QColor(120, 90, 40)
        self.drawBackground = drawBackground
        if imagePath is not None:
            self.setPicture(imagePath)

    def paintEvent(self, e):
        if not self.havePic:
            return super().paintEvent(e)

        painter = QPainter(self)
        painter.setRenderHints(QPainter.Antialiasing)

        pixmap = self.pixmap().scaled(
            self.size()*self.devicePixelRatioF(),
            Qt.IgnoreAspectRatio, Qt.SmoothTransformation)

        path = QPainterPath()
        path.addRoundedRect(QRectF(self.rect()), self.radius, self.radius)

        painter.setClipPath(path)

        if self.drawBackground:
            painter.save()
            painter.setBrush(QColor(0, 0, 0))
            painter.setOpacity(0.8)
            painter.drawRoundedRect(
                QRectF(self.rect()), self.radius, self.radius)
            painter.restore()

        painter.drawPixmap(self.rect(), pixmap)

        if self.borderWidth != 0:
            painter.setPen(
                QPen(self.borderColor, self.borderWidth, Qt.SolidLine))

            painter.drawRoundedRect(
                QRectF(self.rect()), self.radius, self.radius)

    def _acceptAsyncImage(self, image):
        self.setPixmap(QPixmap.fromImage(image))
        self.havePic = True
        self.repaint()

    def _clearAsyncImage(self):
        self.setPixmap(QPixmap())
        self.havePic = False
        self.repaint()

    def setPicture(self, imagePath):
        if isinstance(imagePath, QPixmap):
            self.cancelAsyncImage()
            self.setPixmap(imagePath)
            self.havePic = not imagePath.isNull()
            self.repaint()
            return
        self.setAsyncImage(imagePath)

    def setRedius(self, radius):
        self.radius = radius
        self.repaint()

    def setText(self, text):
        self.cancelAsyncImage()
        return super().setText(text)

    def clear(self):
        self.cancelAsyncImage()
        return super().clear()


class SummonerSpellButton(AsyncImageMixin, QFrame):
    clicked = pyqtSignal(int)

    def __init__(self, imagePath=None, spellId=None, parent=None):
        super().__init__(parent)

        self.image = QPixmap()
        self._initAsyncImage("summoner-spell")

        self.spellId = spellId
        self.radius = 5.0
        self.borderWidth = 2
        self.borderColor = QColor(120, 90, 40)

        self.isPressed = False
        self.isHovered = False
        self.enabled = True
        if imagePath:
            self.setPicture(imagePath)

    def _acceptAsyncImage(self, image):
        self.image = QPixmap.fromImage(image)
        self.update()

    def _clearAsyncImage(self):
        self.image = QPixmap()
        self.update()

    def paintEvent(self, e):
        if not self.image:
            return super().paintEvent(e)

        painter = QPainter(self)
        painter.setRenderHints(QPainter.Antialiasing)

        path = QPainterPath()
        path.addRoundedRect(QRectF(self.rect()), self.radius, self.radius)
        painter.setClipPath(path)

        size = self.size() * self.devicePixelRatioF()
        image = self.image.scaled(size, Qt.AspectRatioMode.KeepAspectRatio,
                                  Qt.TransformationMode.SmoothTransformation)

        if not self.enabled:
            painter.setOpacity(0.5)
        elif self.isPressed:
            painter.setOpacity(0.63)
        elif self.isHovered:
            painter.setOpacity(0.80)
        else:
            painter.setOpacity(1)

        painter.drawPixmap(self.rect(), image)

        painter.setPen(
            QPen(self.borderColor, self.borderWidth, Qt.SolidLine))

        painter.drawRoundedRect(
            QRectF(self.rect()), self.radius, self.radius)

        return super().paintEvent(e)

    def setPicture(self, path):
        self.setAsyncImage(path)

    def setSpellId(self, id):
        self.spellId = id

    def getSpellId(self):
        return self.spellId

    def enterEvent(self, a0: QEvent) -> None:
        self.isHovered = True
        self.update()
        return super().enterEvent(a0)

    def leaveEvent(self, a0: QEvent) -> None:
        self.isHovered = False
        self.update()
        return super().leaveEvent(a0)

    def mousePressEvent(self, a0: QMouseEvent) -> None:
        self.isPressed = True
        self.update()
        return super().mousePressEvent(a0)

    def mouseReleaseEvent(self, a0: QMouseEvent) -> None:
        self.isPressed = False
        self.update()
        ret = super().mouseReleaseEvent(a0)

        if self.enabled:
            self.clicked.emit(self.spellId)

        return ret

    def isEnabled(self):
        return self.isEnabled()

    def setEnabled(self, enabled: bool):
        self.enabled = enabled
        return super().setEnabled(enabled)
