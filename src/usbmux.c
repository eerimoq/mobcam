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

#include "usbmux.h"
#include "plist.h"

#include <string.h>

#include <util/bmem.h>

#ifndef _WIN32
#include <arpa/inet.h>
#endif

#define USBMUX_HEADER_SIZE 16
#define USBMUX_VERSION_PLIST 1
#define USBMUX_TYPE_PLIST 8
/* usbmuxd replies are small; anything larger means the stream is out of sync. */
#define USBMUX_MAX_REPLY_SIZE (4 * 1024 * 1024)
#define USBMUX_CLIENT_NAME "obs-mobcam"

struct usbmux_session {
	mobcam_socket_t sock;
	uint32_t tag;
};

static void write_u32_le(uint8_t *buffer, uint32_t value)
{
	buffer[0] = (uint8_t)value;
	buffer[1] = (uint8_t)(value >> 8);
	buffer[2] = (uint8_t)(value >> 16);
	buffer[3] = (uint8_t)(value >> 24);
}

static uint32_t read_u32_le(const uint8_t *buffer)
{
	return (uint32_t)buffer[0] | ((uint32_t)buffer[1] << 8) | ((uint32_t)buffer[2] << 16) |
	       ((uint32_t)buffer[3] << 24);
}

static void session_close(struct usbmux_session *session)
{
	mobcam_socket_close(session->sock);
	session->sock = MOBCAM_INVALID_SOCKET;
}

static bool session_open(struct usbmux_session *session)
{
	session->sock = mobcam_socket_connect_usbmuxd();
	session->tag = 0;

	return session->sock != MOBCAM_INVALID_SOCKET;
}

/* Starts a request body with the keys usbmuxd expects from every client. */
static void request_begin(struct dstr *body, const char *message_type)
{
	plist_write_begin(body);
	plist_write_string(body, "ClientVersionString", USBMUX_CLIENT_NAME);
	plist_write_string(body, "ProgName", USBMUX_CLIENT_NAME);
	plist_write_integer(body, "kLibUSBMuxVersion", 3);
	plist_write_string(body, "MessageType", message_type);
}

/* Sends a request body and returns the parsed reply, which the caller owns. */
static enum usbmux_result session_request(struct usbmux_session *session, struct dstr *body, struct plist_node **reply,
					  mobcam_abort_cb abort_cb, void *param)
{
	uint8_t header[USBMUX_HEADER_SIZE];
	size_t size = (size_t)body->len;

	session->tag++;

	write_u32_le(&header[0], (uint32_t)(USBMUX_HEADER_SIZE + size));
	write_u32_le(&header[4], USBMUX_VERSION_PLIST);
	write_u32_le(&header[8], USBMUX_TYPE_PLIST);
	write_u32_le(&header[12], session->tag);

	if (!mobcam_socket_write_all(session->sock, header, sizeof(header)) ||
	    !mobcam_socket_write_all(session->sock, body->array, size)) {
		return USBMUX_ERROR;
	}

	enum mobcam_io_result io = mobcam_socket_read_all(session->sock, header, sizeof(header), abort_cb, param);

	if (io == MOBCAM_IO_ABORTED) {
		return USBMUX_ABORTED;
	}

	if (io != MOBCAM_IO_OK) {
		return USBMUX_ERROR;
	}

	uint32_t total = read_u32_le(&header[0]);

	if (total < USBMUX_HEADER_SIZE || total > USBMUX_MAX_REPLY_SIZE) {
		return USBMUX_ERROR;
	}

	size_t payload_size = total - USBMUX_HEADER_SIZE;
	char *payload = bmalloc(payload_size + 1);

	io = mobcam_socket_read_all(session->sock, payload, payload_size, abort_cb, param);

	if (io != MOBCAM_IO_OK) {
		bfree(payload);
		return io == MOBCAM_IO_ABORTED ? USBMUX_ABORTED : USBMUX_ERROR;
	}

	payload[payload_size] = '\0';
	*reply = plist_parse(payload, payload_size);
	bfree(payload);

	return *reply != NULL ? USBMUX_OK : USBMUX_ERROR;
}

static void device_list_add(struct usbmux_device_list *list, uint32_t device_id, const char *serial)
{
	list->devices = brealloc(list->devices, (list->count + 1) * sizeof(*list->devices));
	list->devices[list->count].device_id = device_id;
	list->devices[list->count].serial = bstrdup(serial);
	list->count++;
}

enum usbmux_result usbmux_list_devices(struct usbmux_device_list *list, mobcam_abort_cb abort_cb, void *param)
{
	struct usbmux_session session;
	struct plist_node *reply = NULL;
	struct dstr body = {0};

	memset(list, 0, sizeof(*list));

	if (!session_open(&session)) {
		return USBMUX_NO_DAEMON;
	}

	request_begin(&body, "ListDevices");
	plist_write_end(&body);

	enum usbmux_result result = session_request(&session, &body, &reply, abort_cb, param);

	dstr_free(&body);
	session_close(&session);

	if (result != USBMUX_OK) {
		plist_destroy(reply);
		return result;
	}

	const struct plist_node *devices = plist_get(reply, "DeviceList");

	if (devices != NULL && devices->type == PLIST_TYPE_ARRAY) {
		for (size_t i = 0; i < devices->children_count; i++) {
			const struct plist_node *device = devices->children[i];
			const struct plist_node *properties = plist_get(device, "Properties");
			const char *connection_type = plist_get_string(properties, "ConnectionType");
			const char *serial = plist_get_string(properties, "SerialNumber");
			long long device_id = 0;

			if (!plist_get_integer(device, "DeviceID", &device_id) || serial == NULL) {
				continue;
			}

			/* Wi-Fi paired devices show up here too, and cannot carry this stream. */
			if (connection_type != NULL && strcmp(connection_type, "USB") != 0) {
				continue;
			}

			device_list_add(list, (uint32_t)device_id, serial);
		}
	}

	plist_destroy(reply);

	return list->count > 0 ? USBMUX_OK : USBMUX_NO_DEVICE;
}

void usbmux_device_list_free(struct usbmux_device_list *list)
{
	for (size_t i = 0; i < list->count; i++) {
		bfree(list->devices[i].serial);
	}

	bfree(list->devices);
	memset(list, 0, sizeof(*list));
}

enum usbmux_result usbmux_connect(const char *serial, uint16_t port, mobcam_socket_t *sock, char **serial_out,
				  mobcam_abort_cb abort_cb, void *param)
{
	struct usbmux_device_list list;
	struct usbmux_session session;
	struct plist_node *reply = NULL;
	struct dstr body = {0};
	uint32_t device_id = 0;
	const char *chosen = NULL;

	*sock = MOBCAM_INVALID_SOCKET;
	*serial_out = NULL;

	enum usbmux_result result = usbmux_list_devices(&list, abort_cb, param);

	if (result != USBMUX_OK) {
		usbmux_device_list_free(&list);
		return result;
	}

	for (size_t i = 0; i < list.count; i++) {
		if (serial == NULL || *serial == '\0' || strcmp(list.devices[i].serial, serial) == 0) {
			device_id = list.devices[i].device_id;
			chosen = list.devices[i].serial;
			break;
		}
	}

	if (chosen == NULL) {
		usbmux_device_list_free(&list);
		return USBMUX_NO_DEVICE;
	}

	if (!session_open(&session)) {
		usbmux_device_list_free(&list);
		return USBMUX_NO_DAEMON;
	}

	request_begin(&body, "Connect");
	plist_write_integer(&body, "DeviceID", device_id);
	/* usbmuxd wants the port in network byte order, as an integer. */
	plist_write_integer(&body, "PortNumber", htons(port));
	plist_write_end(&body);

	result = session_request(&session, &body, &reply, abort_cb, param);

	dstr_free(&body);

	if (result == USBMUX_OK) {
		long long number = -1;

		if (!plist_get_integer(reply, "Number", &number) || number != 0) {
			/*
			 * Number 3 is a refused connection, which is what a device
			 * that is not streaming replies. Everything else is a real
			 * failure, but neither is worth a distinct message here.
			 */
			result = USBMUX_REFUSED;
		}
	}

	plist_destroy(reply);

	if (result == USBMUX_OK) {
		*sock = session.sock;
		*serial_out = bstrdup(chosen);
	} else {
		session_close(&session);
	}

	usbmux_device_list_free(&list);

	return result;
}

const char *usbmux_result_message(enum usbmux_result result)
{
	switch (result) {
	case USBMUX_OK:
		return "success";
	case USBMUX_NO_DAEMON:
		return "usbmuxd is not reachable";
	case USBMUX_NO_DEVICE:
		return "no device attached over USB";
	case USBMUX_REFUSED:
		return "the device refused the connection";
	case USBMUX_ABORTED:
		return "aborted";
	default:
		return "usbmuxd communication failed";
	}
}
