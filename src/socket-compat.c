/*
MobCam
Copyright (C) 2026 Erik Moqvist <erik.moqvist@gmail.com>

This program is free software; you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation; either version 2 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License along
with this program. If not, see <https://www.gnu.org/licenses/>
*/

#include "socket-compat.h"

#include <string.h>
#include <stdio.h>

#ifdef _WIN32
#define MOBCAM_USBMUXD_HOST "127.0.0.1"
#define MOBCAM_USBMUXD_PORT 27015
#else
#include <sys/un.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <unistd.h>
#include <poll.h>
#include <errno.h>
#define MOBCAM_USBMUXD_PATH "/var/run/usbmuxd"
#endif

/* How long a blocking read waits before it asks the abort callback again. */
#define MOBCAM_POLL_INTERVAL_MS 100

void mobcam_socket_startup(void)
{
#ifdef _WIN32
	WSADATA data;
	WSAStartup(MAKEWORD(2, 2), &data);
#endif
}

void mobcam_socket_cleanup(void)
{
#ifdef _WIN32
	WSACleanup();
#endif
}

mobcam_socket_t mobcam_socket_connect_usbmuxd(void)
{
#ifdef _WIN32
	struct sockaddr_in addr;
	BOOL nodelay = TRUE;

	mobcam_socket_t sock = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);

	if (sock == MOBCAM_INVALID_SOCKET) {
		return MOBCAM_INVALID_SOCKET;
	}

	memset(&addr, 0, sizeof(addr));
	addr.sin_family = AF_INET;
	addr.sin_port = htons(MOBCAM_USBMUXD_PORT);
	inet_pton(AF_INET, MOBCAM_USBMUXD_HOST, &addr.sin_addr);

	if (connect(sock, (struct sockaddr *)&addr, sizeof(addr)) != 0) {
		mobcam_socket_close(sock);
		return MOBCAM_INVALID_SOCKET;
	}

	setsockopt(sock, IPPROTO_TCP, TCP_NODELAY, (const char *)&nodelay, sizeof(nodelay));

	return sock;
#else
	struct sockaddr_un addr;

	mobcam_socket_t sock = socket(AF_UNIX, SOCK_STREAM, 0);

	if (sock == MOBCAM_INVALID_SOCKET) {
		return MOBCAM_INVALID_SOCKET;
	}

	memset(&addr, 0, sizeof(addr));
	addr.sun_family = AF_UNIX;
	snprintf(addr.sun_path, sizeof(addr.sun_path), "%s", MOBCAM_USBMUXD_PATH);

	if (connect(sock, (struct sockaddr *)&addr, sizeof(addr)) != 0) {
		mobcam_socket_close(sock);
		return MOBCAM_INVALID_SOCKET;
	}

	return sock;
#endif
}

void mobcam_socket_close(mobcam_socket_t sock)
{
	if (sock == MOBCAM_INVALID_SOCKET) {
		return;
	}

#ifdef _WIN32
	closesocket(sock);
#else
	close(sock);
#endif
}

void mobcam_socket_shutdown(mobcam_socket_t sock)
{
	if (sock == MOBCAM_INVALID_SOCKET) {
		return;
	}

#ifdef _WIN32
	shutdown(sock, SD_BOTH);
#else
	shutdown(sock, SHUT_RDWR);
#endif
}

bool mobcam_socket_write_all(mobcam_socket_t sock, const void *data, size_t size)
{
	const char *left = data;

	while (size > 0) {
#ifdef _WIN32
		int written = send(sock, left, (int)size, 0);
#else
		ssize_t written = send(sock, left, size, 0);
#endif

		if (written <= 0) {
#ifndef _WIN32
			if (written < 0 && errno == EINTR) {
				continue;
			}
#endif
			return false;
		}

		left += written;
		size -= (size_t)written;
	}

	return true;
}

/* Returns 1 when readable, 0 on timeout and -1 on error. */
static int mobcam_socket_poll(mobcam_socket_t sock, int timeout_ms)
{
#ifdef _WIN32
	WSAPOLLFD fd;

	fd.fd = sock;
	fd.events = POLLRDNORM;
	fd.revents = 0;

	return WSAPoll(&fd, 1, timeout_ms);
#else
	struct pollfd fd;

	fd.fd = sock;
	fd.events = POLLIN;
	fd.revents = 0;

	int result = poll(&fd, 1, timeout_ms);

	if (result < 0 && errno == EINTR) {
		return 0;
	}

	return result;
#endif
}

enum mobcam_io_result mobcam_socket_read_all(mobcam_socket_t sock, void *data, size_t size, mobcam_abort_cb abort_cb,
					     void *param)
{
	char *left = data;

	while (size > 0) {
		if (abort_cb != NULL && abort_cb(param)) {
			return MOBCAM_IO_ABORTED;
		}

		int ready = mobcam_socket_poll(sock, MOBCAM_POLL_INTERVAL_MS);

		if (ready < 0) {
			return MOBCAM_IO_ERROR;
		}

		if (ready == 0) {
			continue;
		}

#ifdef _WIN32
		int received = recv(sock, left, (int)size, 0);
#else
		ssize_t received = recv(sock, left, size, 0);
#endif

		if (received == 0) {
			return MOBCAM_IO_CLOSED;
		}

		if (received < 0) {
#ifndef _WIN32
			if (errno == EINTR) {
				continue;
			}
#endif
			return MOBCAM_IO_ERROR;
		}

		left += received;
		size -= (size_t)received;
	}

	return MOBCAM_IO_OK;
}

const char *mobcam_socket_error(void)
{
#ifdef _WIN32
	static __declspec(thread) char message[64];

	snprintf(message, sizeof(message), "error %d", WSAGetLastError());

	return message;
#else
	return strerror(errno);
#endif
}
