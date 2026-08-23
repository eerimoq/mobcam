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

#include "plist.h"

#include <stdlib.h>
#include <string.h>

#include <util/bmem.h>

struct parser {
	const char *pos;
	const char *end;
};

static struct plist_node *parse_value(struct parser *parser);

static void skip_space(struct parser *parser)
{
	while (parser->pos < parser->end &&
	       (*parser->pos == ' ' || *parser->pos == '\t' || *parser->pos == '\r' || *parser->pos == '\n')) {
		parser->pos++;
	}
}

static bool looking_at(const struct parser *parser, const char *text)
{
	size_t length = strlen(text);

	return (size_t)(parser->end - parser->pos) >= length && memcmp(parser->pos, text, length) == 0;
}

static void skip_until(struct parser *parser, const char *text)
{
	size_t length = strlen(text);

	while (parser->pos < parser->end) {
		if ((size_t)(parser->end - parser->pos) >= length && memcmp(parser->pos, text, length) == 0) {
			parser->pos += length;
			return;
		}

		parser->pos++;
	}
}

/* Skips the XML declaration, the DOCTYPE and any comments in front of a value. */
static void skip_prolog(struct parser *parser)
{
	for (;;) {
		skip_space(parser);

		if (looking_at(parser, "<?")) {
			skip_until(parser, "?>");
		} else if (looking_at(parser, "<!--")) {
			skip_until(parser, "-->");
		} else if (looking_at(parser, "<!")) {
			skip_until(parser, ">");
		} else {
			return;
		}
	}
}

/* Reads the name of the tag the parser is positioned on, without consuming it. */
static bool peek_tag(const struct parser *parser, char *name, size_t size)
{
	const char *pos = parser->pos;
	size_t length = 0;

	if (pos >= parser->end || *pos != '<') {
		return false;
	}

	pos++;

	if (pos < parser->end && *pos == '/') {
		pos++;
	}

	while (pos < parser->end && *pos != '>' && *pos != '/' && *pos != ' ' && *pos != '\t' && *pos != '\r' &&
	       *pos != '\n') {
		if (length + 1 < size) {
			name[length] = *pos;
		}

		length++;
		pos++;
	}

	name[length < size ? length : size - 1] = '\0';

	return length > 0;
}

/*
 * Consumes the tag the parser is positioned on. Sets *self_closing for tags
 * that carry their own end, such as <true/>.
 */
static bool consume_tag(struct parser *parser, bool *self_closing)
{
	bool empty = false;

	if (parser->pos >= parser->end || *parser->pos != '<') {
		return false;
	}

	while (parser->pos < parser->end && *parser->pos != '>') {
		empty = (*parser->pos == '/');
		parser->pos++;
	}

	if (parser->pos >= parser->end) {
		return false;
	}

	parser->pos++;

	if (self_closing != NULL) {
		*self_closing = empty;
	}

	return true;
}

static void append_entity(struct dstr *out, const char *name, size_t length)
{
	if (length == 3 && memcmp(name, "amp", 3) == 0) {
		dstr_cat_ch(out, '&');
	} else if (length == 2 && memcmp(name, "lt", 2) == 0) {
		dstr_cat_ch(out, '<');
	} else if (length == 2 && memcmp(name, "gt", 2) == 0) {
		dstr_cat_ch(out, '>');
	} else if (length == 4 && memcmp(name, "quot", 4) == 0) {
		dstr_cat_ch(out, '"');
	} else if (length == 4 && memcmp(name, "apos", 4) == 0) {
		dstr_cat_ch(out, '\'');
	} else {
		/* Anything else is left as it was written. */
		dstr_cat_ch(out, '&');
		dstr_ncat(out, name, length);
		dstr_cat_ch(out, ';');
	}
}

/* Reads the text up to the next tag, resolving entities. */
static char *parse_text(struct parser *parser)
{
	struct dstr text = {0};

	while (parser->pos < parser->end && *parser->pos != '<') {
		if (*parser->pos == '&') {
			const char *name = parser->pos + 1;
			const char *semicolon = memchr(name, ';', (size_t)(parser->end - name));

			if (semicolon == NULL) {
				dstr_cat_ch(&text, *parser->pos);
				parser->pos++;
				continue;
			}

			append_entity(&text, name, (size_t)(semicolon - name));
			parser->pos = semicolon + 1;
		} else {
			dstr_cat_ch(&text, *parser->pos);
			parser->pos++;
		}
	}

	if (text.array == NULL) {
		dstr_copy(&text, "");
	}

	return text.array;
}

static struct plist_node *node_create(enum plist_type type)
{
	struct plist_node *node = bzalloc(sizeof(*node));

	node->type = type;

	return node;
}

static void node_add_child(struct plist_node *node, struct plist_node *child)
{
	node->children = brealloc(node->children, (node->children_count + 1) * sizeof(*node->children));
	node->children[node->children_count] = child;
	node->children_count++;
}

static bool parse_dict(struct parser *parser, struct plist_node *node)
{
	for (;;) {
		char name[32];

		skip_space(parser);

		if (!peek_tag(parser, name, sizeof(name))) {
			return false;
		}

		if (looking_at(parser, "</")) {
			return consume_tag(parser, NULL);
		}

		if (strcmp(name, "key") != 0) {
			return false;
		}

		if (!consume_tag(parser, NULL)) {
			return false;
		}

		char *key = parse_text(parser);

		if (!consume_tag(parser, NULL)) {
			bfree(key);
			return false;
		}

		struct plist_node *child = parse_value(parser);

		if (child == NULL) {
			bfree(key);
			return false;
		}

		child->key = key;
		node_add_child(node, child);
	}
}

static bool parse_array(struct parser *parser, struct plist_node *node)
{
	for (;;) {
		char name[32];

		skip_space(parser);

		if (!peek_tag(parser, name, sizeof(name))) {
			return false;
		}

		if (looking_at(parser, "</")) {
			return consume_tag(parser, NULL);
		}

		struct plist_node *child = parse_value(parser);

		if (child == NULL) {
			return false;
		}

		node_add_child(node, child);
	}
}

static struct plist_node *parse_value(struct parser *parser)
{
	char name[32];
	bool self_closing = false;

	skip_space(parser);

	if (!peek_tag(parser, name, sizeof(name)) || !consume_tag(parser, &self_closing)) {
		return NULL;
	}

	if (strcmp(name, "true") == 0 || strcmp(name, "false") == 0) {
		struct plist_node *node = node_create(PLIST_TYPE_BOOL);

		node->boolean = (strcmp(name, "true") == 0);

		if (!self_closing) {
			consume_tag(parser, NULL);
		}

		return node;
	}

	if (strcmp(name, "dict") == 0 || strcmp(name, "array") == 0) {
		bool is_dict = (strcmp(name, "dict") == 0);
		struct plist_node *node = node_create(is_dict ? PLIST_TYPE_DICT : PLIST_TYPE_ARRAY);

		if (self_closing) {
			return node;
		}

		if (!(is_dict ? parse_dict(parser, node) : parse_array(parser, node))) {
			plist_destroy(node);
			return NULL;
		}

		return node;
	}

	enum plist_type type = PLIST_TYPE_OTHER;

	if (strcmp(name, "string") == 0) {
		type = PLIST_TYPE_STRING;
	} else if (strcmp(name, "integer") == 0) {
		type = PLIST_TYPE_INTEGER;
	}

	struct plist_node *node = node_create(type);

	if (self_closing) {
		node->string = bstrdup("");
		return node;
	}

	node->string = parse_text(parser);

	if (type == PLIST_TYPE_INTEGER) {
		node->integer = strtoll(node->string, NULL, 10);
	}

	if (!consume_tag(parser, NULL)) {
		plist_destroy(node);
		return NULL;
	}

	return node;
}

struct plist_node *plist_parse(const char *xml, size_t size)
{
	struct parser parser = {.pos = xml, .end = xml + size};
	char name[32];

	skip_prolog(&parser);

	if (!peek_tag(&parser, name, sizeof(name))) {
		return NULL;
	}

	/* The <plist> element is a wrapper around the one value we want. */
	if (strcmp(name, "plist") == 0) {
		bool self_closing = false;

		if (!consume_tag(&parser, &self_closing) || self_closing) {
			return NULL;
		}
	}

	return parse_value(&parser);
}

void plist_destroy(struct plist_node *node)
{
	if (node == NULL) {
		return;
	}

	for (size_t i = 0; i < node->children_count; i++) {
		plist_destroy(node->children[i]);
	}

	bfree(node->children);
	bfree(node->string);
	bfree(node->key);
	bfree(node);
}

const struct plist_node *plist_get(const struct plist_node *dict, const char *key)
{
	if (dict == NULL || dict->type != PLIST_TYPE_DICT) {
		return NULL;
	}

	for (size_t i = 0; i < dict->children_count; i++) {
		const struct plist_node *child = dict->children[i];

		if (child->key != NULL && strcmp(child->key, key) == 0) {
			return child;
		}
	}

	return NULL;
}

const char *plist_get_string(const struct plist_node *dict, const char *key)
{
	const struct plist_node *node = plist_get(dict, key);

	if (node == NULL || node->type != PLIST_TYPE_STRING) {
		return NULL;
	}

	return node->string;
}

bool plist_get_integer(const struct plist_node *dict, const char *key, long long *value)
{
	const struct plist_node *node = plist_get(dict, key);

	if (node == NULL || node->type != PLIST_TYPE_INTEGER) {
		return false;
	}

	*value = node->integer;

	return true;
}

static void write_escaped(struct dstr *out, const char *value)
{
	for (const char *pos = value; *pos != '\0'; pos++) {
		switch (*pos) {
		case '&':
			dstr_cat(out, "&amp;");
			break;
		case '<':
			dstr_cat(out, "&lt;");
			break;
		case '>':
			dstr_cat(out, "&gt;");
			break;
		default:
			dstr_cat_ch(out, *pos);
			break;
		}
	}
}

void plist_write_begin(struct dstr *out)
{
	dstr_copy(out, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
		       "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" "
		       "\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n"
		       "<plist version=\"1.0\"><dict>");
}

void plist_write_string(struct dstr *out, const char *key, const char *value)
{
	dstr_cat(out, "<key>");
	write_escaped(out, key);
	dstr_cat(out, "</key><string>");
	write_escaped(out, value);
	dstr_cat(out, "</string>");
}

void plist_write_integer(struct dstr *out, const char *key, long long value)
{
	dstr_cat(out, "<key>");
	write_escaped(out, key);
	dstr_catf(out, "</key><integer>%lld</integer>", value);
}

void plist_write_end(struct dstr *out)
{
	dstr_cat(out, "</dict></plist>\n");
}
