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

#include <util/dstr.h>

/*
 * Just enough XML property list support to talk to usbmuxd. It reads the
 * handful of value kinds usbmuxd replies with and writes flat dictionaries of
 * strings and integers, which is all its requests contain.
 */

enum plist_type {
	PLIST_TYPE_DICT,
	PLIST_TYPE_ARRAY,
	PLIST_TYPE_STRING,
	PLIST_TYPE_INTEGER,
	PLIST_TYPE_BOOL,
	PLIST_TYPE_OTHER,
};

struct plist_node {
	enum plist_type type;
	/* Set on the members of a dictionary, NULL everywhere else. */
	char *key;
	char *string;
	long long integer;
	bool boolean;
	struct plist_node **children;
	size_t children_count;
};

struct plist_node *plist_parse(const char *xml, size_t size);
void plist_destroy(struct plist_node *node);

const struct plist_node *plist_get(const struct plist_node *dict, const char *key);
const char *plist_get_string(const struct plist_node *dict, const char *key);
bool plist_get_integer(const struct plist_node *dict, const char *key, long long *value);

void plist_write_begin(struct dstr *out);
void plist_write_string(struct dstr *out, const char *key, const char *value);
void plist_write_integer(struct dstr *out, const char *key, long long value);
void plist_write_end(struct dstr *out);
