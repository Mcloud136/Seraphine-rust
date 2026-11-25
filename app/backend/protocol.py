PROTOCOL_VERSION = 1
MAX_FRAME_SIZE = 1024 * 1024
MAX_REQUEST_BYTES = 256 * 1024
MAX_RESPONSE_BYTES = 512 * 1024
MAX_EVENT_BYTES = 512 * 1024
MAX_IPC_INFLIGHT_BYTES = 4 * 1024 * 1024
MAX_EVENT_QUEUE_BYTES = 1024 * 1024
MAX_PENDING_REQUESTS = 64
MAX_EVENT_QUEUE = 128

CAPABILITY_HEALTH = "health"
CAPABILITY_PROCESS_LIST = "process.list"
CAPABILITY_DIAGNOSTICS = "diagnostics"
CAPABILITY_LCU_READ = "lcu.read"
CAPABILITY_LCU_WRITE = "lcu.write"
CAPABILITY_AUTOMATION = "automation"
CAPABILITY_CHAMPION_ALIAS = "champion_alias"
CAPABILITY_ARAM_BUFF = "aram_buff"

HELLO = "hello"
REQUEST = "request"
RESPONSE = "response"
EVENT = "event"
CANCEL = "cancel"
PING = "ping"
PONG = "pong"
SHUTDOWN = "shutdown"


class ProtocolError(Exception):
    pass


class RpcError(Exception):
    def __init__(self, code, message, retryable=False, source=None):
        super().__init__(message)
        self.code = code
        self.message = message
        self.retryable = retryable
        self.source = source

    @classmethod
    def from_dict(cls, data):
        return cls(
            data.get("code", "RpcError"),
            data.get("message", "Unknown RPC error"),
            bool(data.get("retryable", False)),
            data.get("source"),
        )

    def to_dict(self):
        result = {
            "code": self.code,
            "message": self.message,
            "retryable": self.retryable,
        }
        if self.source is not None:
            result["source"] = self.source
        return result


def message(kind, **fields):
    result = {"version": PROTOCOL_VERSION, "type": kind}
    for key, value in fields.items():
        if value is not None:
            result[key] = value
    return result


def hello(capabilities, app_version, epoch=0):
    return message(
        HELLO,
        epoch=epoch,
        capabilities=list(capabilities),
        app_version=app_version,
    )


def request(request_id, epoch, op, args=None, deadline_ms=None):
    return message(
        REQUEST,
        id=request_id,
        epoch=epoch,
        op=op,
        args={} if args is None else args,
        deadline_ms=deadline_ms,
    )


def response(request_id, epoch, data):
    return message(RESPONSE, id=request_id, epoch=epoch, data=data)


def response_error(request_id, epoch, error):
    if isinstance(error, RpcError):
        error = error.to_dict()
    return message(RESPONSE, id=request_id, epoch=epoch, error=error)


def event(epoch, seq, topic, data):
    return message(EVENT, epoch=epoch, seq=seq, topic=topic, data=data)


def cancel(request_id):
    return message(CANCEL, id=request_id)


def ping(nonce):
    return message(PING, nonce=nonce)


def pong(nonce):
    return message(PONG, nonce=nonce)


def shutdown():
    return message(SHUTDOWN)
