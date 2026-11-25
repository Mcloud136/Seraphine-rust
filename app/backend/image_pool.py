import os
from collections import OrderedDict
from concurrent.futures import ThreadPoolExecutor
from threading import BoundedSemaphore, Lock

from PyQt5.QtCore import QObject, QSize, Qt, QTimer, pyqtSignal, pyqtSlot
from PyQt5.QtGui import QImage, QImageReader, QPixmap


class AsyncImageMixin:
    def _initAsyncImage(self, prefix, target_size=None, use_widget_size=False):
        self._asyncPrefix = prefix
        self._asyncPath = None
        self._asyncKey = None
        self._asyncTargetSize = target_size or QSize(64, 64)
        self._asyncUseWidgetSize = use_widget_size
        self._asyncFailed = False
        self._imagePool = get_image_pool()
        self._imagePool.imageReady.connect(self._onAsyncImageReady)
        self._imagePool.imageFailed.connect(self._onAsyncImageFailed)
        self._imageRetryTimer = QTimer(self)
        self._imageRetryTimer.setSingleShot(True)
        self._imageRetryTimer.timeout.connect(self._retryAsyncImage)

    def setAsyncImage(self, path):
        if isinstance(path, QPixmap):
            self.cancelAsyncImage()
            if not path.isNull():
                self._acceptAsyncImage(path.toImage())
            return
        path = os.fspath(path) if path else ""
        dpr = self.devicePixelRatioF()
        size = (
            self.size()
            if self._asyncUseWidgetSize and self.width() > 0 and self.height() > 0
            else self._asyncTargetSize
        )
        key = (
            f"{self._asyncPrefix}:{path}:{size.width()}:{size.height()}:{dpr:.2f}"
            if path
            else None
        )
        if path == self._asyncPath and key == self._asyncKey and not self._asyncFailed:
            return
        self._imageRetryTimer.stop()
        self._asyncPath = path or None
        self._asyncKey = key
        self._asyncFailed = False
        self._clearAsyncImage()
        if self._asyncPath:
            self._submitAsyncImage()

    def cancelAsyncImage(self):
        self._imageRetryTimer.stop()
        self._asyncPath = None
        self._asyncKey = None
        self._asyncFailed = False
        self._clearAsyncImage()

    def _submitAsyncImage(self):
        if not self._asyncPath or not self._asyncKey:
            return
        size = (
            self.size()
            if self._asyncUseWidgetSize and self.width() > 0 and self.height() > 0
            else self._asyncTargetSize
        )
        if self._imagePool.submit(
            self._asyncKey,
            self._asyncPath,
            size.width(),
            size.height(),
            self.devicePixelRatioF(),
        ):
            self._imageRetryTimer.stop()
        else:
            self._imageRetryTimer.start(50)

    def _retryAsyncImage(self):
        self._submitAsyncImage()

    @pyqtSlot(str, QImage)
    def _onAsyncImageReady(self, key, image):
        if key != self._asyncKey:
            return
        self._imageRetryTimer.stop()
        self._asyncFailed = False
        self._acceptAsyncImage(image)

    @pyqtSlot(str, str)
    def _onAsyncImageFailed(self, key, error):
        if key != self._asyncKey:
            return
        self._imageRetryTimer.stop()
        self._asyncFailed = True
        self._clearAsyncImage()

    def _acceptAsyncImage(self, image):
        raise NotImplementedError

    def _clearAsyncImage(self):
        raise NotImplementedError


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
        self._executor.submit(self._load, key, path, target, dpr)
        return True

    def _load(self, key, path, target, dpr):
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
            target = source_size.scaled(target, Qt.KeepAspectRatio)
            reader.setScaledSize(target)
            image = reader.read()
            if image.isNull():
                raise ValueError(reader.errorString())
            image.setDevicePixelRatio(dpr)

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
