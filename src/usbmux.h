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
#include <stdint.h>

#include "socket-compat.h"

struct usbmux_device {
	uint32_t device_id;
	char *serial;
};

struct usbmux_device_list {
	struct usbmux_device *devices;
	size_t count;
};

enum usbmux_result {
	USBMUX_OK,
	/* usbmuxd is not running, or not installed. */
	USBMUX_NO_DAEMON,
	/* No device is attached over USB, or none with the wanted serial. */
	USBMUX_NO_DEVICE,
	/* The device is there but nothing listens on the port. */
	USBMUX_REFUSED,
	USBMUX_ERROR,
	USBMUX_ABORTED,
};

/* Lists the devices attached over USB. Wi-Fi paired devices are left out. */
enum usbmux_result usbmux_list_devices(struct usbmux_device_list *list, mobcam_abort_cb abort_cb, void *param);
void usbmux_device_list_free(struct usbmux_device_list *list);

/*
 * Opens a connection to a TCP port on the device. Pass an empty or NULL serial
 * to take the first device. On success *sock owns the tunnel and *serial_out
 * names the device it landed on, both owned by the caller.
 */
enum usbmux_result usbmux_connect(const char *serial, uint16_t port, mobcam_socket_t *sock, char **serial_out,
				  mobcam_abort_cb abort_cb, void *param);

const char *usbmux_result_message(enum usbmux_result result);
