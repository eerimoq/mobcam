import json
from base64 import b64encode
from hashlib import sha256
from typing import Any
from uuid import uuid4

from websockets.sync.client import ClientConnection
from websockets.sync.client import connect

RPC_VERSION = 1
OP_IDENTIFY = 1
OP_IDENTIFIED = 2
OP_REQUEST = 6
OP_REQUEST_RESPONSE = 7


def _authentication(password: str, salt: str, challenge: str) -> str:
    secret = b64encode(sha256((password + salt).encode()).digest()).decode()
    return b64encode(sha256((secret + challenge).encode()).digest()).decode()


class ObsWebSocket:
    def __init__(self, host: str, port: int, password: str) -> None:
        self._url = f"ws://{host}:{port}"
        self._password = password
        self._connection: ClientConnection | None = None

    def connect(self) -> None:
        connection = connect(self._url, max_size=None, open_timeout=5)
        try:
            hello = json.loads(connection.recv())["d"]
            identify: dict[str, int | str] = {"rpcVersion": RPC_VERSION, "eventSubscriptions": 0}
            authentication = hello.get("authentication")
            if authentication is not None:
                identify["authentication"] = _authentication(
                    self._password, authentication["salt"], authentication["challenge"]
                )
            connection.send(json.dumps({"op": OP_IDENTIFY, "d": identify}))
            message = json.loads(connection.recv())
            if message["op"] != OP_IDENTIFIED:
                raise Exception(f"Unexpected message {message} when identifying")
        except BaseException:
            connection.close()
            raise
        self._connection = connection

    def close(self) -> None:
        if self._connection is not None:
            self._connection.close()
            self._connection = None

    def request(self, request_type: str, data: dict[str, Any] | None = None) -> dict[str, Any]:
        if self._connection is None:
            raise Exception("Not connected to OBS")
        request_id = str(uuid4())
        self._connection.send(
            json.dumps(
                {
                    "op": OP_REQUEST,
                    "d": {
                        "requestType": request_type,
                        "requestId": request_id,
                        "requestData": data or {},
                    },
                }
            )
        )
        while True:
            message = json.loads(self._connection.recv())
            if message["op"] != OP_REQUEST_RESPONSE or message["d"]["requestId"] != request_id:
                continue
            response = message["d"]
            status = response["requestStatus"]
            if not status["result"]:
                raise Exception(f"{request_type} failed: {status}")
            response_data: dict[str, Any] = response.get("responseData") or {}
            return response_data
