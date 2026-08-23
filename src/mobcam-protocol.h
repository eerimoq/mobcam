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

/*
 * The Moblin USB stream protocol, as described in moblin/docs/usb-protocol.md.
 * Every message is a big endian u32 length covering the type byte and the
 * payload, then the type byte, then the payload.
 */

#define MOBCAM_MESSAGE_HEADER_SIZE 5
#define MOBCAM_MAX_MESSAGE_LENGTH (4 * 1024 * 1024)

#define MOBCAM_MESSAGE_HOST_HELLO 0x01
#define MOBCAM_MESSAGE_DEVICE_HELLO 0x02
#define MOBCAM_MESSAGE_VIDEO_CONFIG 0x03
#define MOBCAM_MESSAGE_VIDEO_FRAME 0x04
#define MOBCAM_MESSAGE_AUDIO_CONFIG 0x05
#define MOBCAM_MESSAGE_AUDIO_FRAME 0x06

#define MOBCAM_PROTOCOL_VERSION 1
#define MOBCAM_HOST_HELLO_SIZE 10

enum mobcam_video_codec {
	MOBCAM_VIDEO_CODEC_H264 = 0,
	MOBCAM_VIDEO_CODEC_HEVC = 1,
};

struct mobcam_device_hello {
	uint8_t version;
	/* Owned by the caller, freed with bfree(). */
	char *name;
	char *app_version;
};

struct mobcam_video_config {
	uint8_t codec;
	uint16_t width;
	uint16_t height;
	/* Points into the message payload: the avcC or hvcC record. */
	const uint8_t *record;
	size_t record_size;
};

struct mobcam_video_frame {
	uint64_t pts_us;
	bool keyframe;
	/* Points into the message payload: one access unit in AVCC form. */
	const uint8_t *data;
	size_t size;
};

/* Writes the hello the host must send first. The buffer holds the whole message. */
void mobcam_pack_host_hello(uint8_t buffer[MOBCAM_HOST_HELLO_SIZE]);

void mobcam_parse_message_header(const uint8_t header[MOBCAM_MESSAGE_HEADER_SIZE], uint32_t *length, uint8_t *type);

bool mobcam_parse_device_hello(const uint8_t *payload, size_t size, struct mobcam_device_hello *hello);
void mobcam_device_hello_free(struct mobcam_device_hello *hello);

bool mobcam_parse_video_config(const uint8_t *payload, size_t size, struct mobcam_video_config *config);
bool mobcam_parse_video_frame(const uint8_t *payload, size_t size, struct mobcam_video_frame *frame);

const char *mobcam_video_codec_name(uint8_t codec);
