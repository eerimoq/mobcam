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

#include "mobcam-source.h"
#include "mobcam-protocol.h"
#include "mobcam-decoder.h"
#include "usbmux.h"
#include "socket-compat.h"

#include <string.h>

#include <util/bmem.h>
#include <util/darray.h>
#include <util/dstr.h>
#include <util/platform.h>
#include <util/threading.h>
#include <plugin-support.h>

#define SETTING_DEVICE "device"
#define SETTING_PORT "port"
#define SETTING_HARDWARE_DECODE "hardware_decode"
#define SETTING_BUFFERING "buffering"
#define SETTING_CLEAR_ON_DISCONNECT "clear_on_disconnect"
#define SETTING_DISCONNECT_WHEN_HIDDEN "disconnect_when_hidden"

#define DEFAULT_PORT 7777
#define RECONNECT_DELAY_MS 1000
/* A timestamp this far from the previous one starts a new timeline. */
#define PTS_DISCONTINUITY_US (5 * 1000 * 1000)

/*
 * Device timestamps run on the phone's clock, which has an unrelated origin to
 * this one, so the first message of every connection anchors a fresh timeline
 * that everything after it is placed on.
 */
struct mobcam_clock {
	bool has_anchor;
	uint64_t first_pts_us;
	uint64_t previous_pts_us;
	uint64_t anchor_ns;
};

struct mobcam_source {
	obs_source_t *source;

	/* Latched settings. Only touched while the worker thread is stopped. */
	char *serial;
	uint16_t port;
	bool hardware_decode;
	bool disconnect_when_hidden;

	/* Read by the worker thread while the OBS thread writes it. */
	bool clear_on_disconnect;

	pthread_t thread;
	bool thread_active;
	bool stopping;
	os_event_t *stop_event;

	struct mobcam_decoder *decoder;
	uint8_t *buffer;
	size_t buffer_capacity;

	/* Video and audio share the device clock, so they share this timeline. */
	struct mobcam_clock clock;

	volatile long width;
	volatile long height;

	/* Keeps a device that is not streaming from filling the log. */
	int reported_failure;
};

/*
 * Devices only tell us their name once a stream is running, so the names seen
 * so far are kept here to label the device list in the properties dialog.
 */
struct name_entry {
	char *serial;
	char *name;
};

static DARRAY(struct name_entry) name_cache;
static pthread_mutex_t name_cache_mutex;

void mobcam_source_global_init(void)
{
	da_init(name_cache);
	pthread_mutex_init(&name_cache_mutex, NULL);
}

void mobcam_source_global_free(void)
{
	for (size_t i = 0; i < name_cache.num; i++) {
		bfree(name_cache.array[i].serial);
		bfree(name_cache.array[i].name);
	}

	da_free(name_cache);
	pthread_mutex_destroy(&name_cache_mutex);
}

static void name_cache_set(const char *serial, const char *name)
{
	if (serial == NULL || name == NULL || *name == '\0') {
		return;
	}

	pthread_mutex_lock(&name_cache_mutex);

	for (size_t i = 0; i < name_cache.num; i++) {
		if (strcmp(name_cache.array[i].serial, serial) == 0) {
			bfree(name_cache.array[i].name);
			name_cache.array[i].name = bstrdup(name);
			pthread_mutex_unlock(&name_cache_mutex);
			return;
		}
	}

	struct name_entry entry = {.serial = bstrdup(serial), .name = bstrdup(name)};

	da_push_back(name_cache, &entry);
	pthread_mutex_unlock(&name_cache_mutex);
}

/* Returns a copy of the cached name, or NULL when the device is unknown. */
static char *name_cache_get(const char *serial)
{
	char *name = NULL;

	pthread_mutex_lock(&name_cache_mutex);

	for (size_t i = 0; i < name_cache.num; i++) {
		if (strcmp(name_cache.array[i].serial, serial) == 0) {
			name = bstrdup(name_cache.array[i].name);
			break;
		}
	}

	pthread_mutex_unlock(&name_cache_mutex);

	return name;
}

static bool source_aborting(void *param)
{
	struct mobcam_source *context = param;

	return os_atomic_load_bool(&context->stopping);
}

static void source_clear_video(struct mobcam_source *context)
{
	if (os_atomic_load_bool(&context->clear_on_disconnect)) {
		obs_source_output_video(context->source, NULL);
		os_atomic_set_long(&context->width, 0);
		os_atomic_set_long(&context->height, 0);
	}
}

/* Places one device timestamp on this connection's timeline. */
static uint64_t clock_timestamp(struct mobcam_clock *clock, uint64_t pts_us)
{
	uint64_t distance = pts_us > clock->previous_pts_us ? pts_us - clock->previous_pts_us
							    : clock->previous_pts_us - pts_us;

	if (!clock->has_anchor || distance > PTS_DISCONTINUITY_US) {
		clock->has_anchor = true;
		clock->first_pts_us = pts_us;
		clock->anchor_ns = os_gettime_ns();
	} else if (pts_us < clock->first_pts_us) {
		/*
		 * The stream that anchored the timeline had started a little
		 * later than the other one. Move the origin back rather than
		 * anchor again, so what has already gone out stays where it is.
		 */
		clock->anchor_ns -= (clock->first_pts_us - pts_us) * 1000;
		clock->first_pts_us = pts_us;
	}

	clock->previous_pts_us = pts_us;

	return clock->anchor_ns + (pts_us - clock->first_pts_us) * 1000;
}

static void on_decoded_frame(void *param, struct obs_source_frame *frame, uint64_t pts_us)
{
	struct mobcam_source *context = param;

	frame->timestamp = clock_timestamp(&context->clock, pts_us);

	os_atomic_set_long(&context->width, (long)frame->width);
	os_atomic_set_long(&context->height, (long)frame->height);

	obs_source_output_video(context->source, frame);
}

static void on_decoded_audio(void *param, struct obs_source_audio *audio, uint64_t pts_us)
{
	struct mobcam_source *context = param;

	audio->timestamp = clock_timestamp(&context->clock, pts_us);

	obs_source_output_audio(context->source, audio);
}

static uint8_t *source_buffer(struct mobcam_source *context, size_t size)
{
	size_t needed = size + MOBCAM_INPUT_PADDING;

	if (context->buffer_capacity < needed) {
		context->buffer = brealloc(context->buffer, needed);
		context->buffer_capacity = needed;
	}

	memset(context->buffer + size, 0, MOBCAM_INPUT_PADDING);

	return context->buffer;
}

static bool handle_message(struct mobcam_source *context, uint8_t type, const uint8_t *payload, size_t size,
			   const char *serial)
{
	switch (type) {
	case MOBCAM_MESSAGE_DEVICE_HELLO: {
		struct mobcam_device_hello hello;

		if (!mobcam_parse_device_hello(payload, size, &hello)) {
			obs_log(LOG_WARNING, "malformed device hello");
			return false;
		}

		obs_log(LOG_INFO, "connected to %s (Moblin %s) on %s", hello.name, hello.app_version, serial);
		name_cache_set(serial, hello.name);
		mobcam_device_hello_free(&hello);

		return true;
	}
	case MOBCAM_MESSAGE_VIDEO_CONFIG: {
		struct mobcam_video_config config;

		if (!mobcam_parse_video_config(payload, size, &config)) {
			obs_log(LOG_WARNING, "malformed video config");
			return false;
		}

		return mobcam_decoder_configure_video(context->decoder, &config);
	}
	case MOBCAM_MESSAGE_VIDEO_FRAME: {
		struct mobcam_video_frame frame;

		if (!mobcam_parse_video_frame(payload, size, &frame)) {
			obs_log(LOG_WARNING, "malformed video frame");
			return false;
		}

		return mobcam_decoder_decode_video(context->decoder, &frame, on_decoded_frame, context);
	}
	case MOBCAM_MESSAGE_AUDIO_CONFIG: {
		struct mobcam_audio_config config;

		if (!mobcam_parse_audio_config(payload, size, &config)) {
			obs_log(LOG_WARNING, "malformed audio config");
			return false;
		}

		/* Audio the decoder will not take is no reason to lose the video. */
		mobcam_decoder_configure_audio(context->decoder, &config);

		return true;
	}
	case MOBCAM_MESSAGE_AUDIO_FRAME: {
		struct mobcam_audio_frame frame;

		if (!mobcam_parse_audio_frame(payload, size, &frame)) {
			obs_log(LOG_WARNING, "malformed audio frame");
			return false;
		}

		mobcam_decoder_decode_audio(context->decoder, &frame, on_decoded_audio, context);

		return true;
	}
	default:
		/* Unknown messages are skipped. */
		return true;
	}
}

/* Reads messages until the connection ends or the source is stopped. */
static void source_stream(struct mobcam_source *context, mobcam_socket_t sock, const char *serial)
{
	uint8_t hello[MOBCAM_HOST_HELLO_SIZE];

	mobcam_pack_host_hello(hello);

	if (!mobcam_socket_write_all(sock, hello, sizeof(hello))) {
		obs_log(LOG_WARNING, "failed to say hello to %s", serial);
		return;
	}

	memset(&context->clock, 0, sizeof(context->clock));
	mobcam_decoder_reset(context->decoder);

	for (;;) {
		uint8_t header[MOBCAM_MESSAGE_HEADER_SIZE];
		uint32_t length;
		uint8_t type;

		if (mobcam_socket_read_all(sock, header, sizeof(header), source_aborting, context) != MOBCAM_IO_OK) {
			break;
		}

		mobcam_parse_message_header(header, &length, &type);

		if (length < 1 || length > MOBCAM_MAX_MESSAGE_LENGTH) {
			obs_log(LOG_WARNING, "bad message length %u", length);
			break;
		}

		size_t payload_size = length - 1;
		uint8_t *payload = source_buffer(context, payload_size);

		if (mobcam_socket_read_all(sock, payload, payload_size, source_aborting, context) != MOBCAM_IO_OK) {
			break;
		}

		if (!handle_message(context, type, payload, payload_size, serial)) {
			break;
		}
	}
}

static void source_connect(struct mobcam_source *context)
{
	mobcam_socket_t sock = MOBCAM_INVALID_SOCKET;
	char *serial = NULL;

	enum usbmux_result result =
		usbmux_connect(context->serial, context->port, &sock, &serial, source_aborting, context);

	if (result != USBMUX_OK) {
		/*
		 * A phone that is attached but not streaming refuses the
		 * connection once a second, so each reason is only logged when
		 * it changes.
		 */
		if (result != USBMUX_ABORTED && context->reported_failure != (int)result) {
			context->reported_failure = (int)result;
			obs_log(LOG_INFO, "not connected: %s", usbmux_result_message(result));
		}

		return;
	}

	context->reported_failure = -1;

	source_stream(context, sock, serial);

	mobcam_socket_close(sock);

	if (!os_atomic_load_bool(&context->stopping)) {
		obs_log(LOG_INFO, "disconnected from %s", serial);
	}

	bfree(serial);
	source_clear_video(context);
}

static void *source_thread(void *param)
{
	struct mobcam_source *context = param;

	os_set_thread_name("mobcam");

	while (!os_atomic_load_bool(&context->stopping)) {
		source_connect(context);

		if (os_atomic_load_bool(&context->stopping)) {
			break;
		}

		os_event_timedwait(context->stop_event, RECONNECT_DELAY_MS);
	}

	return NULL;
}

static void source_start(struct mobcam_source *context)
{
	if (context->thread_active || context->decoder == NULL) {
		return;
	}

	os_atomic_set_bool(&context->stopping, false);
	os_event_reset(context->stop_event);
	context->reported_failure = -1;

	if (pthread_create(&context->thread, NULL, source_thread, context) != 0) {
		obs_log(LOG_ERROR, "failed to start the receive thread");
		return;
	}

	context->thread_active = true;
}

static void source_stop(struct mobcam_source *context)
{
	if (!context->thread_active) {
		return;
	}

	os_atomic_set_bool(&context->stopping, true);
	os_event_signal(context->stop_event);
	pthread_join(context->thread, NULL);
	context->thread_active = false;

	source_clear_video(context);
}

static const char *mobcam_source_get_name(void *type_data)
{
	UNUSED_PARAMETER(type_data);

	return obs_module_text("MobCam");
}

static void mobcam_source_update(void *data, obs_data_t *settings)
{
	struct mobcam_source *context = data;
	const char *serial = obs_data_get_string(settings, SETTING_DEVICE);
	uint16_t port = (uint16_t)obs_data_get_int(settings, SETTING_PORT);
	bool hardware_decode = obs_data_get_bool(settings, SETTING_HARDWARE_DECODE);
	bool buffering = obs_data_get_bool(settings, SETTING_BUFFERING);
	bool disconnect_when_hidden = obs_data_get_bool(settings, SETTING_DISCONNECT_WHEN_HIDDEN);

	os_atomic_set_bool(&context->clear_on_disconnect, obs_data_get_bool(settings, SETTING_CLEAR_ON_DISCONNECT));
	obs_source_set_async_unbuffered(context->source, !buffering);

	bool restart = (port != context->port) || (hardware_decode != context->hardware_decode) ||
		       (strcmp(serial, context->serial == NULL ? "" : context->serial) != 0);

	if (restart) {
		/*
		 * The decoder is only safe to retune while the receive thread
		 * is stopped, and the reconnect is what brings the config
		 * message that opens it again.
		 */
		source_stop(context);
		bfree(context->serial);
		context->serial = bstrdup(serial);
		context->port = port;
		context->hardware_decode = hardware_decode;
		mobcam_decoder_set_hardware(context->decoder, hardware_decode);
	}

	context->disconnect_when_hidden = disconnect_when_hidden;

	if (!disconnect_when_hidden || obs_source_showing(context->source)) {
		source_start(context);
	} else {
		source_stop(context);
	}
}

static void *mobcam_source_create(obs_data_t *settings, obs_source_t *source)
{
	struct mobcam_source *context = bzalloc(sizeof(*context));

	context->source = source;
	context->serial = bstrdup("");
	context->port = DEFAULT_PORT;
	context->decoder = mobcam_decoder_create();
	context->reported_failure = -1;
	os_event_init(&context->stop_event, OS_EVENT_TYPE_MANUAL);

	if (context->decoder == NULL) {
		obs_log(LOG_ERROR, "failed to create the decoder");
	}

	mobcam_source_update(context, settings);

	return context;
}

static void mobcam_source_destroy(void *data)
{
	struct mobcam_source *context = data;

	source_stop(context);
	mobcam_decoder_destroy(context->decoder);
	os_event_destroy(context->stop_event);
	bfree(context->buffer);
	bfree(context->serial);
	bfree(context);
}

static void mobcam_source_show(void *data)
{
	struct mobcam_source *context = data;

	if (context->disconnect_when_hidden) {
		source_start(context);
	}
}

static void mobcam_source_hide(void *data)
{
	struct mobcam_source *context = data;

	if (context->disconnect_when_hidden) {
		source_stop(context);
	}
}

static uint32_t mobcam_source_get_width(void *data)
{
	struct mobcam_source *context = data;

	return (uint32_t)os_atomic_load_long(&context->width);
}

static uint32_t mobcam_source_get_height(void *data)
{
	struct mobcam_source *context = data;

	return (uint32_t)os_atomic_load_long(&context->height);
}

static void mobcam_source_defaults(obs_data_t *settings)
{
	obs_data_set_default_string(settings, SETTING_DEVICE, "");
	obs_data_set_default_int(settings, SETTING_PORT, DEFAULT_PORT);
	obs_data_set_default_bool(settings, SETTING_HARDWARE_DECODE, false);
	obs_data_set_default_bool(settings, SETTING_BUFFERING, false);
	obs_data_set_default_bool(settings, SETTING_CLEAR_ON_DISCONNECT, true);
	obs_data_set_default_bool(settings, SETTING_DISCONNECT_WHEN_HIDDEN, false);
}

/* Keeps a wedged usbmuxd from hanging the thread that asked it something. */
struct deadline {
	uint64_t end_ns;
};

static bool deadline_expired(void *param)
{
	const struct deadline *deadline = param;

	return os_gettime_ns() >= deadline->end_ns;
}

static void fill_device_list(obs_property_t *list)
{
	struct usbmux_device_list devices;
	struct deadline deadline = {.end_ns = os_gettime_ns() + 2000000000ULL};

	obs_property_list_clear(list);
	obs_property_list_add_string(list, obs_module_text("Device.Automatic"), "");

	if (usbmux_list_devices(&devices, deadline_expired, &deadline) != USBMUX_OK) {
		usbmux_device_list_free(&devices);
		return;
	}

	for (size_t i = 0; i < devices.count; i++) {
		const char *serial = devices.devices[i].serial;
		char *name = name_cache_get(serial);
		struct dstr label = {0};

		if (name != NULL) {
			dstr_printf(&label, "%s (%s)", name, serial);
		} else {
			dstr_copy(&label, serial);
		}

		obs_property_list_add_string(list, label.array, serial);
		dstr_free(&label);
		bfree(name);
	}

	usbmux_device_list_free(&devices);
}

static bool refresh_devices_clicked(obs_properties_t *props, obs_property_t *property, void *data)
{
	UNUSED_PARAMETER(property);
	UNUSED_PARAMETER(data);

	fill_device_list(obs_properties_get(props, SETTING_DEVICE));

	return true;
}

static obs_properties_t *mobcam_source_properties(void *data)
{
	UNUSED_PARAMETER(data);

	obs_properties_t *props = obs_properties_create();

	obs_property_t *list = obs_properties_add_list(props, SETTING_DEVICE, obs_module_text("Device"),
						       OBS_COMBO_TYPE_LIST, OBS_COMBO_FORMAT_STRING);

	fill_device_list(list);

	obs_properties_add_button2(props, "refresh", obs_module_text("RefreshDevices"), refresh_devices_clicked, NULL);
	obs_properties_add_int(props, SETTING_PORT, obs_module_text("Port"), 1, 65535, 1);
	obs_properties_add_bool(props, SETTING_HARDWARE_DECODE, obs_module_text("HardwareDecode"));
	obs_properties_add_bool(props, SETTING_BUFFERING, obs_module_text("Buffering"));
	obs_properties_add_bool(props, SETTING_CLEAR_ON_DISCONNECT, obs_module_text("ClearOnDisconnect"));
	obs_properties_add_bool(props, SETTING_DISCONNECT_WHEN_HIDDEN, obs_module_text("DisconnectWhenHidden"));

	return props;
}

struct obs_source_info mobcam_source_info = {
	.id = "mobcam_source",
	.type = OBS_SOURCE_TYPE_INPUT,
	.output_flags = OBS_SOURCE_ASYNC_VIDEO | OBS_SOURCE_AUDIO | OBS_SOURCE_DO_NOT_DUPLICATE,
	.icon_type = OBS_ICON_TYPE_CAMERA,
	.get_name = mobcam_source_get_name,
	.create = mobcam_source_create,
	.destroy = mobcam_source_destroy,
	.update = mobcam_source_update,
	.show = mobcam_source_show,
	.hide = mobcam_source_hide,
	.get_width = mobcam_source_get_width,
	.get_height = mobcam_source_get_height,
	.get_defaults = mobcam_source_defaults,
	.get_properties = mobcam_source_properties,
};
