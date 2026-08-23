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
#include <libavutil/channel_layout.h>
#include <libavutil/pixdesc.h>
#include <libavutil/samplefmt.h>

#if MOBCAM_INPUT_PADDING < AV_INPUT_BUFFER_PADDING_SIZE
#error "MOBCAM_INPUT_PADDING is smaller than what this libavcodec requires"
#endif

/* One decoder and the config message it was opened for. */
struct mobcam_stream {
	AVCodecContext *context;
	AVPacket *packet;
	AVFrame *frame;

	uint8_t codec;
	uint8_t *record;
	size_t record_size;
};

struct mobcam_decoder {
	struct mobcam_stream video;
	struct mobcam_stream audio;

	/* Video is held back until the first keyframe after every open. */
	bool got_keyframe;
	/* Unsupported formats are logged once instead of once per frame. */
	int logged_pixel_format;
	int logged_sample_format;
	int logged_channels;
};

static bool stream_init(struct mobcam_stream *stream)
{
	stream->packet = av_packet_alloc();
	stream->frame = av_frame_alloc();

	return stream->packet != NULL && stream->frame != NULL;
}

static void stream_close(struct mobcam_stream *stream)
{
	avcodec_free_context(&stream->context);
	bfree(stream->record);
	stream->record = NULL;
	stream->record_size = 0;
}

static void stream_free(struct mobcam_stream *stream)
{
	stream_close(stream);
	av_packet_free(&stream->packet);
	av_frame_free(&stream->frame);
}

/* True when the stream is already decoding exactly this configuration. */
static bool stream_configured(const struct mobcam_stream *stream, uint8_t codec, const uint8_t *record,
			      size_t record_size)
{
	return stream->context != NULL && stream->codec == codec && stream->record_size == record_size &&
	       memcmp(stream->record, record, record_size) == 0;
}

/*
 * Allocates a context holding the configuration record as extradata. The caller
 * fills in the stream specific fields and hands it to stream_open().
 */
static bool stream_begin(struct mobcam_stream *stream, const AVCodec *codec, const uint8_t *record, size_t record_size)
{
	stream->context = avcodec_alloc_context3(codec);

	if (stream->context == NULL) {
		return false;
	}

	if (record_size == 0) {
		return true;
	}

	stream->context->extradata = av_mallocz(record_size + AV_INPUT_BUFFER_PADDING_SIZE);

	if (stream->context->extradata == NULL) {
		stream_close(stream);
		return false;
	}

	memcpy(stream->context->extradata, record, record_size);
	stream->context->extradata_size = (int)record_size;

	return true;
}

static bool stream_open(struct mobcam_stream *stream, const AVCodec *codec, uint8_t wire_codec, const uint8_t *record,
			size_t record_size)
{
	if (avcodec_open2(stream->context, codec, NULL) < 0) {
		stream_close(stream);
		return false;
	}

	stream->codec = wire_codec;
	stream->record = bmemdup(record, record_size);
	stream->record_size = record_size;

	return true;
}

/*
 * Hands one access unit to the decoder. The receive buffer is padded, so it can
 * be referenced in place; avcodec_send_packet() copies what it needs to keep.
 */
static bool stream_send(struct mobcam_stream *stream, const uint8_t *data, size_t size, int64_t pts, bool keyframe)
{
	AVPacket *packet = stream->packet;

	av_packet_unref(packet);
	packet->data = (uint8_t *)data;
	packet->size = (int)size;
	packet->pts = pts;
	packet->dts = pts;

	if (keyframe) {
		packet->flags |= AV_PKT_FLAG_KEY;
	}

	int result = avcodec_send_packet(stream->context, packet);

	packet->data = NULL;
	packet->size = 0;

	return result >= 0 || result == AVERROR(EAGAIN);
}

struct mobcam_decoder *mobcam_decoder_create(void)
{
	struct mobcam_decoder *decoder = bzalloc(sizeof(*decoder));

	decoder->logged_pixel_format = AV_PIX_FMT_NONE;
	decoder->logged_sample_format = AV_SAMPLE_FMT_NONE;
	decoder->logged_channels = -1;

	if (!stream_init(&decoder->video) || !stream_init(&decoder->audio)) {
		mobcam_decoder_destroy(decoder);
		return NULL;
	}

	return decoder;
}

void mobcam_decoder_destroy(struct mobcam_decoder *decoder)
{
	if (decoder == NULL) {
		return;
	}

	stream_free(&decoder->video);
	stream_free(&decoder->audio);
	bfree(decoder);
}

void mobcam_decoder_reset(struct mobcam_decoder *decoder)
{
	if (decoder->video.context != NULL) {
		avcodec_flush_buffers(decoder->video.context);
	}

	if (decoder->audio.context != NULL) {
		avcodec_flush_buffers(decoder->audio.context);
	}

	decoder->got_keyframe = false;
}

bool mobcam_decoder_configure_video(struct mobcam_decoder *decoder, const struct mobcam_video_config *config)
{
	struct mobcam_stream *stream = &decoder->video;
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

	if (stream_configured(stream, config->codec, config->record, config->record_size)) {
		return true;
	}

	stream_close(stream);
	decoder->got_keyframe = false;

	const AVCodec *codec = avcodec_find_decoder(codec_id);

	if (codec == NULL) {
		obs_log(LOG_ERROR, "no %s decoder available", mobcam_video_codec_name(config->codec));
		return false;
	}

	/*
	 * The avcC or hvcC record goes in unmodified, which is what makes the
	 * decoder expect the length prefixed access units that arrive on the
	 * wire rather than Annex-B.
	 */
	if (!stream_begin(stream, codec, config->record, config->record_size)) {
		return false;
	}

	stream->context->width = config->width;
	stream->context->height = config->height;
	stream->context->flags |= AV_CODEC_FLAG_LOW_DELAY;
	/* Frame threading would add a frame of latency to a live camera. */
	stream->context->thread_type = FF_THREAD_SLICE;

	if (!stream_open(stream, codec, config->codec, config->record, config->record_size)) {
		obs_log(LOG_ERROR, "failed to open the %s decoder", mobcam_video_codec_name(config->codec));
		return false;
	}

	decoder->logged_pixel_format = AV_PIX_FMT_NONE;

	obs_log(LOG_INFO, "decoding %s %ux%u", mobcam_video_codec_name(config->codec), config->width, config->height);

	return true;
}

bool mobcam_decoder_configure_audio(struct mobcam_decoder *decoder, const struct mobcam_audio_config *config)
{
	struct mobcam_stream *stream = &decoder->audio;
	enum AVCodecID codec_id;

	switch (config->codec) {
	case MOBCAM_AUDIO_CODEC_AAC_LC:
		codec_id = AV_CODEC_ID_AAC;
		break;
	default:
		obs_log(LOG_WARNING, "unsupported audio codec %u", config->codec);
		return false;
	}

	if (stream_configured(stream, config->codec, config->record, config->record_size)) {
		return true;
	}

	stream_close(stream);

	const AVCodec *codec = avcodec_find_decoder(codec_id);

	if (codec == NULL) {
		obs_log(LOG_ERROR, "no %s decoder available", mobcam_audio_codec_name(config->codec));
		return false;
	}

	/*
	 * The AudioSpecificConfig goes in as extradata, so the decoder knows
	 * what the raw access units on the wire hold without an ADTS header in
	 * front of each one.
	 */
	if (!stream_begin(stream, codec, config->record, config->record_size)) {
		return false;
	}

	stream->context->sample_rate = (int)config->sample_rate;
	av_channel_layout_default(&stream->context->ch_layout, config->channels);

	if (!stream_open(stream, codec, config->codec, config->record, config->record_size)) {
		obs_log(LOG_ERROR, "failed to open the %s decoder", mobcam_audio_codec_name(config->codec));
		return false;
	}

	decoder->logged_sample_format = AV_SAMPLE_FMT_NONE;
	decoder->logged_channels = -1;

	obs_log(LOG_INFO, "decoding %s %u Hz %u channel", mobcam_audio_codec_name(config->codec), config->sample_rate,
		config->channels);

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
		if (decoder->logged_pixel_format != source->format) {
			decoder->logged_pixel_format = source->format;
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

static enum audio_format audio_format_from_sample_format(int format)
{
	switch (format) {
	case AV_SAMPLE_FMT_U8:
		return AUDIO_FORMAT_U8BIT;
	case AV_SAMPLE_FMT_S16:
		return AUDIO_FORMAT_16BIT;
	case AV_SAMPLE_FMT_S32:
		return AUDIO_FORMAT_32BIT;
	case AV_SAMPLE_FMT_FLT:
		return AUDIO_FORMAT_FLOAT;
	case AV_SAMPLE_FMT_U8P:
		return AUDIO_FORMAT_U8BIT_PLANAR;
	case AV_SAMPLE_FMT_S16P:
		return AUDIO_FORMAT_16BIT_PLANAR;
	case AV_SAMPLE_FMT_S32P:
		return AUDIO_FORMAT_32BIT_PLANAR;
	case AV_SAMPLE_FMT_FLTP:
		return AUDIO_FORMAT_FLOAT_PLANAR;
	default:
		return AUDIO_FORMAT_UNKNOWN;
	}
}

static enum speaker_layout speakers_from_channels(int channels)
{
	switch (channels) {
	case 1:
		return SPEAKERS_MONO;
	case 2:
		return SPEAKERS_STEREO;
	case 3:
		return SPEAKERS_2POINT1;
	case 4:
		return SPEAKERS_4POINT0;
	case 5:
		return SPEAKERS_4POINT1;
	case 6:
		return SPEAKERS_5POINT1;
	case 8:
		return SPEAKERS_7POINT1;
	default:
		return SPEAKERS_UNKNOWN;
	}
}

static bool audio_to_obs(struct mobcam_decoder *decoder, const AVFrame *source, struct obs_source_audio *audio)
{
	memset(audio, 0, sizeof(*audio));

	audio->format = audio_format_from_sample_format(source->format);

	if (audio->format == AUDIO_FORMAT_UNKNOWN) {
		if (decoder->logged_sample_format != source->format) {
			decoder->logged_sample_format = source->format;
			obs_log(LOG_WARNING, "unsupported sample format %s", av_get_sample_fmt_name(source->format));
		}

		return false;
	}

	audio->speakers = speakers_from_channels(source->ch_layout.nb_channels);

	if (audio->speakers == SPEAKERS_UNKNOWN) {
		if (decoder->logged_channels != source->ch_layout.nb_channels) {
			decoder->logged_channels = source->ch_layout.nb_channels;
			obs_log(LOG_WARNING, "unsupported channel count %d", source->ch_layout.nb_channels);
		}

		return false;
	}

	size_t planes = get_audio_planes(audio->format, audio->speakers);

	for (size_t i = 0; i < planes && i < MAX_AV_PLANES; i++) {
		audio->data[i] = source->extended_data[i];
	}

	audio->frames = (uint32_t)source->nb_samples;
	audio->samples_per_sec = (uint32_t)source->sample_rate;

	return true;
}

bool mobcam_decoder_decode_video(struct mobcam_decoder *decoder, const struct mobcam_video_frame *video_frame,
				 mobcam_decoder_video_cb callback, void *param)
{
	struct mobcam_stream *stream = &decoder->video;

	if (stream->context == NULL) {
		return true;
	}

	if (!decoder->got_keyframe) {
		if (!video_frame->keyframe) {
			return true;
		}

		decoder->got_keyframe = true;
	}

	if (!stream_send(stream, video_frame->data, video_frame->size, (int64_t)video_frame->pts_us,
			 video_frame->keyframe)) {
		obs_log(LOG_WARNING, "failed to decode a frame, flushing the decoder");
		avcodec_flush_buffers(stream->context);
		decoder->got_keyframe = false;
		return true;
	}

	for (;;) {
		int result = avcodec_receive_frame(stream->context, stream->frame);

		if (result == AVERROR(EAGAIN) || result == AVERROR_EOF) {
			break;
		}

		if (result < 0) {
			return false;
		}

		struct obs_source_frame frame;

		if (frame_to_obs(decoder, stream->frame, &frame)) {
			callback(param, &frame, (uint64_t)stream->frame->pts);
		}

		av_frame_unref(stream->frame);
	}

	return true;
}

void mobcam_decoder_decode_audio(struct mobcam_decoder *decoder, const struct mobcam_audio_frame *audio_frame,
				 mobcam_decoder_audio_cb callback, void *param)
{
	struct mobcam_stream *stream = &decoder->audio;

	if (stream->context == NULL) {
		return;
	}

	if (!stream_send(stream, audio_frame->data, audio_frame->size, (int64_t)audio_frame->pts_us, true)) {
		obs_log(LOG_WARNING, "failed to decode audio, flushing the decoder");
		avcodec_flush_buffers(stream->context);
		return;
	}

	for (;;) {
		int result = avcodec_receive_frame(stream->context, stream->frame);

		if (result == AVERROR(EAGAIN) || result == AVERROR_EOF) {
			break;
		}

		if (result < 0) {
			obs_log(LOG_WARNING, "failed to decode audio, flushing the decoder");
			avcodec_flush_buffers(stream->context);
			break;
		}

		struct obs_source_audio audio;

		if (audio_to_obs(decoder, stream->frame, &audio)) {
			callback(param, &audio, (uint64_t)stream->frame->pts);
		}

		av_frame_unref(stream->frame);
	}
}
