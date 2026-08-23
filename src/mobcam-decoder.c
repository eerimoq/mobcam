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

#include "mobcam-decoder.h"

#include <string.h>

#include <util/bmem.h>
#include <plugin-support.h>

#include <libavcodec/avcodec.h>
#include <libavutil/pixdesc.h>

#if MOBCAM_INPUT_PADDING < AV_INPUT_BUFFER_PADDING_SIZE
#error "MOBCAM_INPUT_PADDING is smaller than what this libavcodec requires"
#endif

struct mobcam_decoder {
	AVCodecContext *context;
	AVPacket *packet;
	AVFrame *frame;

	uint8_t codec;
	uint8_t *record;
	size_t record_size;

	/* Output is held back until the first keyframe after every open. */
	bool got_keyframe;
	/* Unsupported pixel formats are logged once instead of once per frame. */
	int logged_format;
};

struct mobcam_decoder *mobcam_decoder_create(void)
{
	struct mobcam_decoder *decoder = bzalloc(sizeof(*decoder));

	decoder->packet = av_packet_alloc();
	decoder->frame = av_frame_alloc();
	decoder->logged_format = AV_PIX_FMT_NONE;

	if (decoder->packet == NULL || decoder->frame == NULL) {
		mobcam_decoder_destroy(decoder);
		return NULL;
	}

	return decoder;
}

static void decoder_close(struct mobcam_decoder *decoder)
{
	avcodec_free_context(&decoder->context);
	bfree(decoder->record);
	decoder->record = NULL;
	decoder->record_size = 0;
	decoder->got_keyframe = false;
}

void mobcam_decoder_destroy(struct mobcam_decoder *decoder)
{
	if (decoder == NULL) {
		return;
	}

	decoder_close(decoder);
	av_packet_free(&decoder->packet);
	av_frame_free(&decoder->frame);
	bfree(decoder);
}

bool mobcam_decoder_ready(const struct mobcam_decoder *decoder)
{
	return decoder->context != NULL;
}

void mobcam_decoder_reset(struct mobcam_decoder *decoder)
{
	if (decoder->context != NULL) {
		avcodec_flush_buffers(decoder->context);
	}

	decoder->got_keyframe = false;
}

bool mobcam_decoder_configure(struct mobcam_decoder *decoder, const struct mobcam_video_config *config)
{
	enum AVCodecID codec_id;

	switch (config->codec) {
	case MOBCAM_VIDEO_CODEC_H264:
		codec_id = AV_CODEC_ID_H264;
		break;
	case MOBCAM_VIDEO_CODEC_HEVC:
		codec_id = AV_CODEC_ID_HEVC;
		break;
	default:
		obs_log(LOG_WARNING, "unsupported video codec %u", config->codec);
		return false;
	}

	if (decoder->context != NULL && decoder->codec == config->codec &&
	    decoder->record_size == config->record_size &&
	    memcmp(decoder->record, config->record, config->record_size) == 0) {
		return true;
	}

	decoder_close(decoder);

	const AVCodec *codec = avcodec_find_decoder(codec_id);

	if (codec == NULL) {
		obs_log(LOG_ERROR, "no %s decoder available", mobcam_video_codec_name(config->codec));
		return false;
	}

	decoder->context = avcodec_alloc_context3(codec);

	if (decoder->context == NULL) {
		return false;
	}

	/*
	 * The avcC or hvcC record goes in unmodified, which is what makes the
	 * decoder expect the length prefixed access units that arrive on the
	 * wire rather than Annex-B.
	 */
	if (config->record_size > 0) {
		decoder->context->extradata = av_mallocz(config->record_size + AV_INPUT_BUFFER_PADDING_SIZE);

		if (decoder->context->extradata == NULL) {
			decoder_close(decoder);
			return false;
		}

		memcpy(decoder->context->extradata, config->record, config->record_size);
		decoder->context->extradata_size = (int)config->record_size;
	}

	decoder->context->width = config->width;
	decoder->context->height = config->height;
	decoder->context->flags |= AV_CODEC_FLAG_LOW_DELAY;
	/* Frame threading would add a frame of latency to a live camera. */
	decoder->context->thread_type = FF_THREAD_SLICE;

	if (avcodec_open2(decoder->context, codec, NULL) < 0) {
		obs_log(LOG_ERROR, "failed to open the %s decoder", mobcam_video_codec_name(config->codec));
		decoder_close(decoder);
		return false;
	}

	decoder->codec = config->codec;
	decoder->record = bmemdup(config->record, config->record_size);
	decoder->record_size = config->record_size;
	decoder->logged_format = AV_PIX_FMT_NONE;

	obs_log(LOG_INFO, "decoding %s %ux%u", mobcam_video_codec_name(config->codec), config->width, config->height);

	return true;
}

static enum video_format video_format_from_pixel_format(int format, bool *full_range)
{
	switch (format) {
	case AV_PIX_FMT_YUVJ420P:
		*full_range = true;
		return VIDEO_FORMAT_I420;
	case AV_PIX_FMT_YUV420P:
		return VIDEO_FORMAT_I420;
	case AV_PIX_FMT_NV12:
		return VIDEO_FORMAT_NV12;
	case AV_PIX_FMT_YUV420P10LE:
		return VIDEO_FORMAT_I010;
	case AV_PIX_FMT_P010LE:
		return VIDEO_FORMAT_P010;
	case AV_PIX_FMT_YUV444P:
		return VIDEO_FORMAT_I444;
	case AV_PIX_FMT_YUV422P:
		return VIDEO_FORMAT_I422;
	default:
		return VIDEO_FORMAT_NONE;
	}
}

static enum video_colorspace colorspace_from_frame(const AVFrame *frame)
{
	switch (frame->colorspace) {
	case AVCOL_SPC_BT470BG:
	case AVCOL_SPC_SMPTE170M:
		return VIDEO_CS_601;
	case AVCOL_SPC_BT2020_NCL:
		return frame->color_trc == AVCOL_TRC_ARIB_STD_B67 ? VIDEO_CS_2100_HLG : VIDEO_CS_2100_PQ;
	default:
		return VIDEO_CS_709;
	}
}

static uint8_t trc_from_frame(const AVFrame *frame)
{
	switch (frame->color_trc) {
	case AVCOL_TRC_SMPTE2084:
		return VIDEO_TRC_PQ;
	case AVCOL_TRC_ARIB_STD_B67:
		return VIDEO_TRC_HLG;
	default:
		return VIDEO_TRC_DEFAULT;
	}
}

static bool frame_to_obs(struct mobcam_decoder *decoder, const AVFrame *source, struct obs_source_frame *frame)
{
	bool full_range = (source->color_range == AVCOL_RANGE_JPEG);

	memset(frame, 0, sizeof(*frame));

	frame->format = video_format_from_pixel_format(source->format, &full_range);

	if (frame->format == VIDEO_FORMAT_NONE) {
		if (decoder->logged_format != source->format) {
			decoder->logged_format = source->format;
			obs_log(LOG_WARNING, "unsupported pixel format %s", av_get_pix_fmt_name(source->format));
		}

		return false;
	}

	for (size_t i = 0; i < MAX_AV_PLANES; i++) {
		frame->data[i] = source->data[i];
		frame->linesize[i] = (uint32_t)source->linesize[i];
	}

	frame->width = (uint32_t)source->width;
	frame->height = (uint32_t)source->height;
	frame->full_range = full_range;
	frame->trc = trc_from_frame(source);

	enum video_range_type range = full_range ? VIDEO_RANGE_FULL : VIDEO_RANGE_PARTIAL;

	video_format_get_parameters_for_format(colorspace_from_frame(source), range, frame->format, frame->color_matrix,
					       frame->color_range_min, frame->color_range_max);

	return true;
}

bool mobcam_decoder_decode(struct mobcam_decoder *decoder, const struct mobcam_video_frame *video_frame,
			   mobcam_decoder_frame_cb callback, void *param)
{
	if (decoder->context == NULL) {
		return true;
	}

	if (!decoder->got_keyframe) {
		if (!video_frame->keyframe) {
			return true;
		}

		decoder->got_keyframe = true;
	}

	AVPacket *packet = decoder->packet;

	av_packet_unref(packet);
	/*
	 * The receive buffer is padded, so the access unit can be referenced in
	 * place. avcodec_send_packet() copies what it needs to keep.
	 */
	packet->data = (uint8_t *)video_frame->data;
	packet->size = (int)video_frame->size;
	packet->pts = (int64_t)video_frame->pts_us;
	packet->dts = packet->pts;

	if (video_frame->keyframe) {
		packet->flags |= AV_PKT_FLAG_KEY;
	}

	int result = avcodec_send_packet(decoder->context, packet);

	packet->data = NULL;
	packet->size = 0;

	if (result < 0 && result != AVERROR(EAGAIN)) {
		obs_log(LOG_WARNING, "failed to decode a frame, flushing the decoder");
		avcodec_flush_buffers(decoder->context);
		decoder->got_keyframe = false;
		return true;
	}

	for (;;) {
		result = avcodec_receive_frame(decoder->context, decoder->frame);

		if (result == AVERROR(EAGAIN) || result == AVERROR_EOF) {
			break;
		}

		if (result < 0) {
			return false;
		}

		struct obs_source_frame frame;

		if (frame_to_obs(decoder, decoder->frame, &frame)) {
			callback(param, &frame, (uint64_t)decoder->frame->pts);
		}

		av_frame_unref(decoder->frame);
	}

	return true;
}
