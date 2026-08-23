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

#include "mobcam-protocol.h"

#include <string.h>

#include <obs-module.h>
#include <util/bmem.h>

static uint16_t read_u16_be(const uint8_t *buffer)
{
	return (uint16_t)(((uint16_t)buffer[0] << 8) | buffer[1]);
}

static uint32_t read_u32_be(const uint8_t *buffer)
{
	return ((uint32_t)buffer[0] << 24) | ((uint32_t)buffer[1] << 16) | ((uint32_t)buffer[2] << 8) |
	       (uint32_t)buffer[3];
}

static uint64_t read_u64_be(const uint8_t *buffer)
{
	return ((uint64_t)read_u32_be(buffer) << 32) | read_u32_be(buffer + 4);
}

void mobcam_pack_host_hello(uint8_t buffer[MOBCAM_HOST_HELLO_SIZE])
{
	/* Length covers the type byte, "MOBL" and the version byte. */
	buffer[0] = 0;
	buffer[1] = 0;
	buffer[2] = 0;
	buffer[3] = 6;
	buffer[4] = MOBCAM_MESSAGE_HOST_HELLO;
	buffer[5] = 'M';
	buffer[6] = 'O';
	buffer[7] = 'B';
	buffer[8] = 'L';
	buffer[9] = MOBCAM_PROTOCOL_VERSION;
}

void mobcam_parse_message_header(const uint8_t header[MOBCAM_MESSAGE_HEADER_SIZE], uint32_t *length, uint8_t *type)
{
	*length = read_u32_be(header);
	*type = header[4];
}

bool mobcam_parse_device_hello(const uint8_t *payload, size_t size, struct mobcam_device_hello *hello)
{
	memset(hello, 0, sizeof(*hello));

	if (size < 5) {
		return false;
	}

	uint32_t json_size = read_u32_be(payload + 1);

	if (json_size > size - 5) {
		return false;
	}

	hello->version = payload[0];

	char *json = bmalloc(json_size + 1);

	memcpy(json, payload + 5, json_size);
	json[json_size] = '\0';

	obs_data_t *data = obs_data_create_from_json(json);

	bfree(json);

	if (data == NULL) {
		return false;
	}

	hello->name = bstrdup(obs_data_get_string(data, "name"));
	hello->app_version = bstrdup(obs_data_get_string(data, "version"));
	obs_data_release(data);

	return true;
}

void mobcam_device_hello_free(struct mobcam_device_hello *hello)
{
	bfree(hello->name);
	bfree(hello->app_version);
	memset(hello, 0, sizeof(*hello));
}

bool mobcam_parse_video_config(const uint8_t *payload, size_t size, struct mobcam_video_config *config)
{
	memset(config, 0, sizeof(*config));

	if (size < 9) {
		return false;
	}

	uint32_t record_size = read_u32_be(payload + 5);

	if (record_size > size - 9) {
		return false;
	}

	config->codec = payload[0];
	config->width = read_u16_be(payload + 1);
	config->height = read_u16_be(payload + 3);
	config->record = payload + 9;
	config->record_size = record_size;

	return true;
}

bool mobcam_parse_video_frame(const uint8_t *payload, size_t size, struct mobcam_video_frame *frame)
{
	memset(frame, 0, sizeof(*frame));

	if (size < 9) {
		return false;
	}

	frame->pts_us = read_u64_be(payload);
	frame->keyframe = (payload[8] & 1) != 0;
	frame->data = payload + 9;
	frame->size = size - 9;

	return true;
}

const char *mobcam_video_codec_name(uint8_t codec)
{
	switch (codec) {
	case MOBCAM_VIDEO_CODEC_H264:
		return "H.264";
	case MOBCAM_VIDEO_CODEC_HEVC:
		return "HEVC";
	default:
		return "unknown";
	}
}
