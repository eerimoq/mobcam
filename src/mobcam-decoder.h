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
#include <stdint.h>

#include <obs-module.h>

#include "mobcam-protocol.h"

struct mobcam_decoder;

/*
 * Access units are handed to the decoder in place, so the buffer they live in
 * must have this many zeroed bytes after them. Kept free of libavcodec here and
 * checked against AV_INPUT_BUFFER_PADDING_SIZE where the decoder is built.
 */
#define MOBCAM_INPUT_PADDING 64

/*
 * The frame handed to these callbacks borrows the decoder's memory and is only
 * valid until the callback returns. Its timestamp is left for the caller to
 * fill in from pts_us.
 */
typedef void (*mobcam_decoder_video_cb)(void *param, struct obs_source_frame *frame, uint64_t pts_us);
typedef void (*mobcam_decoder_audio_cb)(void *param, struct obs_source_audio *audio, uint64_t pts_us);

struct mobcam_decoder *mobcam_decoder_create(void);
void mobcam_decoder_destroy(struct mobcam_decoder *decoder);

/*
 * Chooses whether video is decoded on the GPU. Hardware decoding is a request
 * rather than a demand: a machine with nothing that decodes the codec falls
 * back to software instead of failing. Takes effect the next time the video
 * decoder is opened, so it must be set while nothing is decoding.
 */
void mobcam_decoder_set_hardware(struct mobcam_decoder *decoder, bool hardware);

/*
 * Opens, or reopens, a decoder for a config message. Reopening is skipped when
 * the codec and the configuration record are unchanged, so calling these for
 * every config message is free.
 */
bool mobcam_decoder_configure_video(struct mobcam_decoder *decoder, const struct mobcam_video_config *config);
bool mobcam_decoder_configure_audio(struct mobcam_decoder *decoder, const struct mobcam_audio_config *config);

/*
 * Drops whatever the decoders were in the middle of and waits for a keyframe
 * again. Called when a connection starts, since the frames on either side of it
 * belong to different encoder sessions.
 */
void mobcam_decoder_reset(struct mobcam_decoder *decoder);

/*
 * Video is what the source is for, so a stream it cannot decode ends the
 * connection and another one is tried. Audio it cannot decode is dropped
 * instead, leaving the video running, which is why only the video half of this
 * pair can tell the caller to give up.
 */
bool mobcam_decoder_decode_video(struct mobcam_decoder *decoder, const struct mobcam_video_frame *frame,
				 mobcam_decoder_video_cb callback, void *param);
void mobcam_decoder_decode_audio(struct mobcam_decoder *decoder, const struct mobcam_audio_frame *frame,
				 mobcam_decoder_audio_cb callback, void *param);
