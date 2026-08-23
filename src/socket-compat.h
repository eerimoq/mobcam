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

#pragma once

#include <stdbool.h>
#include <stddef.h>

#ifdef _WIN32
#include <winsock2.h>
#include <ws2tcpip.h>
typedef SOCKET mobcam_socket_t;
#define MOBCAM_INVALID_SOCKET INVALID_SOCKET
#else
#include <sys/socket.h>
typedef int mobcam_socket_t;
#define MOBCAM_INVALID_SOCKET (-1)
#endif

/*
 * Called while blocked on the socket. Returning true makes the pending
 * operation give up, which is how a source that is being stopped or destroyed
 * gets its worker thread back.
 */
typedef bool (*mobcam_abort_cb)(void *param);

/* Results shared by every blocking socket helper below. */
enum mobcam_io_result {
	MOBCAM_IO_OK,
	MOBCAM_IO_ABORTED,
	MOBCAM_IO_CLOSED,
	MOBCAM_IO_ERROR,
};

void mobcam_socket_startup(void);
void mobcam_socket_cleanup(void);

/* Connects to usbmuxd: a unix socket on macOS and Linux, TCP on Windows. */
mobcam_socket_t mobcam_socket_connect_usbmuxd(void);

void mobcam_socket_close(mobcam_socket_t sock);

/* Wakes up a thread blocked in mobcam_socket_read_all() on this socket. */
void mobcam_socket_shutdown(mobcam_socket_t sock);

bool mobcam_socket_write_all(mobcam_socket_t sock, const void *data, size_t size);

enum mobcam_io_result mobcam_socket_read_all(mobcam_socket_t sock, void *data, size_t size, mobcam_abort_cb abort_cb,
					     void *param);

const char *mobcam_socket_error(void);
