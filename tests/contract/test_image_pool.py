import os
import tempfile
import time
import unittest

from PyQt5.QtCore import QTimer
from PyQt5.QtGui import QColor, QImage, QPixmap
from PyQt5.QtWidgets import QApplication

from app.backend.image_pool import ImagePool
from app.components.champion_icon_widget import (
    AsyncPixmapLabel,
    RoundedLabel,
    RoundIcon,
    SummonerSpellButton,
    TopRoundedLabel,
)


class ImagePoolTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.directory.name, "icon.png")
        image = QImage(128, 128, QImage.Format_ARGB32)
        image.fill(QColor(20, 120, 220))
        self.assertTrue(image.save(self.path))

    def tearDown(self):
        self.directory.cleanup()

    def wait_for(self, predicate, timeout=3.0):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.app.processEvents()
            if predicate():
                return True
            time.sleep(0.01)
        return False

    def make_image(self, name, color):
        path = os.path.join(self.directory.name, name)
        image = QImage(128, 128, QImage.Format_ARGB32)
        image.fill(color)
        self.assertTrue(image.save(path))
        return path

    def test_decodes_and_scales_off_gui_thread(self):
        pool = ImagePool()
        received = []

        def on_ready(key, image):
            received.append((key, image))
            pool.close()

        pool.imageReady.connect(on_ready)
        self.assertTrue(pool.submit("test", self.path, 32, 32, 2.0))
        QTimer.singleShot(5000, pool.close)
        for _ in range(100):
            self.app.processEvents()
            if received:
                break
            time.sleep(0.02)
        self.assertEqual(len(received), 1)
        self.assertEqual(received[0][1].size().width(), 64)
        self.assertEqual(received[0][1].size().height(), 64)
        self.assertEqual(received[0][1].devicePixelRatio(), 2.0)

    def test_rounded_label_clears_old_image_when_key_changes(self):
        second = self.make_image("second.png", QColor(220, 40, 20))
        label = RoundedLabel(self.path, radius=0, borderWidth=0)
        self.assertTrue(self.wait_for(lambda: label.havePic))

        label.setPicture(second)
        self.assertFalse(label.havePic)
        self.assertTrue(label.pixmap() is None or label.pixmap().isNull())
        self.assertTrue(self.wait_for(lambda: label.havePic))
        self.assertEqual(label.pixmap().toImage().pixelColor(0, 0), QColor(220, 40, 20))
        label.close()

    def test_rounded_label_clears_failure_and_ignores_stale_result(self):
        label = RoundedLabel(self.path, radius=0, borderWidth=0)
        self.assertTrue(self.wait_for(lambda: label.havePic))
        stale_key = label._asyncKey

        label.setPicture(os.path.join(self.directory.name, "missing.png"))
        self.assertFalse(label.havePic)
        label._imagePool.imageReady.emit(stale_key, QImage(8, 8, QImage.Format_ARGB32))
        self.assertFalse(label.havePic)
        self.assertTrue(self.wait_for(lambda: label._asyncFailed))
        self.assertTrue(label.pixmap() is None or label.pixmap().isNull())

        label.setText("no image")
        self.assertIsNone(label._asyncKey)
        self.assertTrue(label.pixmap() is None or label.pixmap().isNull())
        label.close()

    def test_async_controls_share_failure_and_stale_result_guards(self):
        controls = [
            RoundIcon(self.path, 32),
            TopRoundedLabel(),
            SummonerSpellButton(self.path, 1),
            AsyncPixmapLabel(self.path),
        ]
        for control in controls:
            control.setFixedSize(32, 32)
            if control._asyncKey is None:
                control.setAsyncImage(self.path)
            self.assertTrue(self.wait_for(lambda item=control: item._asyncKey is not None))
            key = control._asyncKey
            control.setAsyncImage(os.path.join(self.directory.name, "missing.png"))
            control._imagePool.imageReady.emit(key, QImage())
            self.assertTrue(self.wait_for(lambda item=control: item._asyncFailed))
            control.close()

    def test_pixmap_input_and_clear_cancel_async_loading(self):
        pixmap = QPixmap(16, 16)
        pixmap.fill(QColor(20, 120, 220))
        label = RoundedLabel()
        label.setPicture(pixmap)
        self.assertTrue(label.havePic)
        label.setAsyncImage(self.path)
        label.clear()
        self.assertIsNone(label._asyncKey)
        self.assertTrue(label.pixmap() is None or label.pixmap().isNull())

        button = SummonerSpellButton()
        button.setPicture(pixmap)
        self.assertFalse(button.image.isNull())
        button.setPicture(None)
        self.assertTrue(button.image.isNull())
        label.close()
        button.close()

    def test_image_pool_reuses_cached_key(self):
        pool = ImagePool()
        received = []
        pool.imageReady.connect(lambda key, image: received.append((key, image)))
        self.assertTrue(pool.submit("cached", self.path, 16, 16))
        self.assertTrue(self.wait_for(lambda: bool(received)))
        count = len(received)
        self.assertTrue(pool.submit("cached", self.path, 16, 16))
        self.assertEqual(len(received), count + 1)
        pool.close()


if __name__ == "__main__":
    unittest.main()
