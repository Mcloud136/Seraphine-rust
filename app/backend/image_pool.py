import os
from collections import OrderedDict
from concurrent.futures import ThreadPoolExecutor
from threading import BoundedSemaphore, Lock

from PyQt5.QtCore import QObject, QSize, pyqtSignal
from PyQt5.QtGui import QImage, QImageReader


class ImagePool(QObject):
    imageReady = pyqtSignal(str, QImage)
    imageFailed = pyqtSignal(str, str)

    MAX_QUEUE = 64
    MAX_WORKERS = 2
    MAX_CACHE_ENTRIES = 64
    MAX_CACHE_BYTES = 32 * 1024 * 1024
    MAX_SOURCE_BYTES = 16 * 1024 * 1024
    MAX_SOURCE_PIXELS = 64 * 1024 * 1024

    def __init__(self, parent=None):
        super().__init__(parent)
        self._executor = ThreadPoolExecutor(
            max_workers=self.MAX_WORKERS,
            thread_name_prefix="seraphine-image",
        )
        self._slots = BoundedSemaphore(self.MAX_QUEUE)
        self._lock = Lock()
        self._cache = OrderedDict()
        self._cache_bytes = 0
        self._inflight = set()

    def submit(self, key, path, width, height, dpr=1.0):
        if width <= 0 or height <= 0:
            return False
        with self._lock:
            cached = self._cache.get(key)
            if cached is not None:
                self._cache.move_to_end(key)
                self.imageReady.emit(key, cached)
                return True
            if key in self._inflight or not self._slots.acquire(blocking=False):
                return False
            self._inflight.add(key)
        target = QSize(
            max(1, int(width * dpr)),
            max(1, int(height * dpr)),
        )
        self._executor.submit(self._load, key, path, target)
        return True

    def _load(self, key, path, target):
        try:
            if not os.path.isfile(path):
                raise FileNotFoundError(path)
            if os.path.getsize(path) > self.MAX_SOURCE_BYTES:
                raise ValueError("image source exceeds byte budget")

            reader = QImageReader(path)
            reader.setDecideFormatFromContent(True)
            reader.setAutoTransform(True)
            source_size = reader.size()
            if source_size.width() * source_size.height() > self.MAX_SOURCE_PIXELS:
                raise ValueError("image source exceeds pixel budget")
            reader.setScaledSize(target)
            image = reader.read()
            if image.isNull():
                raise ValueError(reader.errorString())

            self._cache_put(key, image)
            self.imageReady.emit(key, image)
        except BaseException as error:
            self.imageFailed.emit(key, str(error))
        finally:
            with self._lock:
                self._inflight.discard(key)
            self._slots.release()

    def _cache_put(self, key, image):
        with self._lock:
            old = self._cache.pop(key, None)
            if old is not None:
                self._cache_bytes -= old.byteCount()
            self._cache[key] = image
            self._cache_bytes += image.byteCount()
            while (
                len(self._cache) > self.MAX_CACHE_ENTRIES
                or self._cache_bytes > self.MAX_CACHE_BYTES
            ):
                _, evicted = self._cache.popitem(last=False)
                self._cache_bytes -= evicted.byteCount()

    def clear(self):
        with self._lock:
            self._cache.clear()
            self._cache_bytes = 0

    def close(self):
        self._executor.shutdown(wait=False)


_pool = None


def get_image_pool():
    global _pool
    if _pool is None:
        _pool = ImagePool()
    return _pool
