import os
import tempfile
import time
import unittest

from PyQt5.QtCore import QTimer
from PyQt5.QtGui import QColor, QImage
from PyQt5.QtWidgets import QApplication

from app.backend.image_pool import ImagePool


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


if __name__ == "__main__":
    unittest.main()
