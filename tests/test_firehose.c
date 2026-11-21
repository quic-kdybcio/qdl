// SPDX-License-Identifier: BSD-3-Clause
/*
 * Copyright (c) Qualcomm Technologies, Inc. and/or its subsidiaries.
 */
/*
 * Unit tests for the Firehose XML response parsers. They are file-local
 * in firehose.c, so the module is #included directly. Only the response
 * parsers are exercised; the rest of firehose.c (and its device / vip /
 * ufs dependencies) is dropped at link time via --gc-sections, so only
 * the ux_* logging stubs are needed.
 */
#include <setjmp.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>

#include <cmocka.h>
#include <libxml/tree.h>

#include "firehose.c"

void ux_err(const char *fmt, ...) { (void)fmt; }
void ux_log(const char *fmt, ...) { (void)fmt; }
void ux_log_tag(enum ux_tag tag, const char *fmt, ...) { (void)tag; (void)fmt; }
void ux_debug_tag(enum ux_tag tag, const char *fmt, ...) { (void)tag; (void)fmt; }

static xmlNode *mk(const char *name)
{
	return xmlNewNode(NULL, (const xmlChar *)name);
}

static void attr(xmlNode *n, const char *k, const char *v)
{
	xmlNewProp(n, (const xmlChar *)k, (const xmlChar *)v);
}

/* --- firehose_generic_parser --- */

static void test_generic_ack_nak_log(void **state)
{
	bool rawmode = false;
	xmlNode *n;
	(void)state;

	n = mk("response");
	attr(n, "value", "ACK");
	assert_int_equal(firehose_generic_parser(n, NULL, &rawmode), FIREHOSE_ACK);
	xmlFreeNode(n);

	n = mk("response");
	attr(n, "value", "NAK");
	assert_int_equal(firehose_generic_parser(n, NULL, &rawmode), FIREHOSE_NAK);
	xmlFreeNode(n);

	n = mk("log");
	attr(n, "value", "some log line");
	assert_int_equal(firehose_generic_parser(n, NULL, &rawmode), -EAGAIN);
	xmlFreeNode(n);
}

static void test_generic_rawmode(void **state)
{
	bool rawmode = false;
	xmlNode *n;
	(void)state;

	n = mk("response");
	attr(n, "value", "ACK");
	attr(n, "rawmode", "true");
	assert_int_equal(firehose_generic_parser(n, NULL, &rawmode), FIREHOSE_ACK);
	assert_true(rawmode);
	xmlFreeNode(n);
}

static void test_generic_missing_value(void **state)
{
	bool rawmode = false;
	xmlNode *n = mk("response");
	(void)state;

	assert_int_equal(firehose_generic_parser(n, NULL, &rawmode), -EINVAL);
	xmlFreeNode(n);
}

/* --- firehose_configure_response_parser --- */

static void test_configure_ack(void **state)
{
	size_t max_size = 0;
	bool rawmode = false;
	xmlNode *n = mk("response");
	(void)state;

	attr(n, "value", "ACK");
	attr(n, "MaxPayloadSizeToTargetInBytes", "1048576");
	attr(n, "MaxPayloadSizeToTargetInBytesSupported", "2097152");

	assert_int_equal(firehose_configure_response_parser(n, &max_size, &rawmode),
			 FIREHOSE_ACK);
	/* An ACK advertises the larger supported size. */
	assert_int_equal(max_size, 2097152);
	xmlFreeNode(n);
}

static void test_configure_nak_negotiates(void **state)
{
	size_t max_size = 0;
	bool rawmode = false;
	xmlNode *n = mk("response");
	(void)state;

	/*
	 * A configure NAK is the payload-size negotiation, not a failure:
	 * the device rejects the host's proposal and answers with the size
	 * it accepts, which the caller re-sends. Treating the NAK as an
	 * error makes the handshake loop forever on such devices.
	 */
	attr(n, "value", "NAK");
	attr(n, "MaxPayloadSizeToTargetInBytes", "1048576");

	assert_int_equal(firehose_configure_response_parser(n, &max_size, &rawmode),
			 FIREHOSE_ACK);
	assert_int_equal(max_size, 1048576);
	xmlFreeNode(n);
}

static void test_configure_ack_without_supported(void **state)
{
	size_t max_size = 0;
	bool rawmode = false;
	xmlNode *n = mk("response");
	(void)state;

	/* An ACK that omits the "Supported" size is malformed. */
	attr(n, "value", "ACK");
	attr(n, "MaxPayloadSizeToTargetInBytes", "1048576");

	assert_true(firehose_configure_response_parser(n, &max_size, &rawmode) < 0);
	xmlFreeNode(n);
}

static void test_configure_log_and_missing(void **state)
{
	size_t max_size = 0;
	bool rawmode = false;
	xmlNode *n;
	(void)state;

	n = mk("log");
	attr(n, "value", "startup log");
	assert_int_equal(firehose_configure_response_parser(n, &max_size, &rawmode), -EAGAIN);
	xmlFreeNode(n);

	n = mk("response");
	assert_int_equal(firehose_configure_response_parser(n, &max_size, &rawmode), -EINVAL);
	xmlFreeNode(n);
}

/* --- firehose_getstorageinfo_parser --- */

static void getstorageinfo_log(const char *value, struct firehose_getsize_args *args)
{
	bool rawmode = false;
	xmlNode *n = mk("log");

	attr(n, "value", value);
	assert_int_equal(firehose_getstorageinfo_parser(n, args, &rawmode), 0);
	xmlFreeNode(n);
}

static void test_getstorageinfo_json(void **state)
{
	struct firehose_getsize_args args = {};
	bool rawmode = false;
	xmlNode *n;
	(void)state;

	getstorageinfo_log("INFO: Device Total Logical Blocks: 0x1d00000", &args);
	assert_int_equal(args.num_sectors, 0);

	getstorageinfo_log("INFO: {\"storage_info\": {\"total_blocks\":30431232, "
			   "\"block_size\":4096, \"page_size\":4096, \"num_physical\":6, "
			   "\"mem_type\":\"UFS\"}}", &args);
	assert_int_equal(args.num_sectors, 30431232);
	assert_int_equal(args.sector_size, 4096);

	n = mk("response");
	attr(n, "value", "ACK");
	assert_int_equal(firehose_getstorageinfo_parser(n, &args, &rawmode), 1);
	xmlFreeNode(n);
}

/*
 * Reply of the mahua-crd NVMe programmer: "spec_version":ae80 makes the
 * blob invalid JSON, but the geometry must still be picked up.
 */
static void test_getstorageinfo_invalid_json(void **state)
{
	struct firehose_getsize_args args = {};
	(void)state;

	getstorageinfo_log("INFO: {\"storage_info\": {\"total_blocks\":2000409264, "
			   "\"block_size\":512, \"page_size\":512, \"num_physical\":1, "
			   "\"spec_version\":ae80, \"manufacturer_id\":475798620, "
			   "\"serial_num\":4294967295, \"fw_version\":\"61020A5\","
			   "\"mem_type\":\"NVMe\",\"prod_name\":\"HFS001TFM9X186N                \"}}",
			   &args);
	assert_int_equal(args.num_sectors, 2000409264);
	assert_int_equal(args.sector_size, 512);
}

/* Only direct members of storage_info count, not keys nested or quoted */
static void test_getstorageinfo_invalid_json_scope(void **state)
{
	struct firehose_getsize_args args = {};
	(void)state;

	getstorageinfo_log("INFO: {\"other\":{\"block_size\":4096}, \"storage_info\": "
			   "{\"total_blocks\":10, \"nested\":{\"block_size\":4096}, "
			   "\"prod_name\":\"\\\"block_size\\\":4096\", "
			   "\"block_size\":512, \"spec_version\":ae80}, "
			   "\"block_size\":4096}", &args);
	assert_int_equal(args.num_sectors, 10);
	assert_int_equal(args.sector_size, 512);
}

/*
 * Malformed numbers in a non-JSON blob must not be taken as geometry, and a
 * failed scan must leave the previous geometry untouched.
 */
static void test_getstorageinfo_invalid_json_bad_numbers(void **state)
{
	static const char * const bad[] = {
		"INFO: {\"storage_info\": {\"total_blocks\":-1, "
		"\"block_size\":512, \"spec_version\":ae80}}",
		"INFO: {\"storage_info\": {\"total_blocks\":"
		"999999999999999999999999999999, \"block_size\":512, "
		"\"spec_version\":ae80}}",
		"INFO: {\"storage_info\": {\"total_blocks\":1e6, "
		"\"block_size\":512, \"spec_version\":ae80}}",
		"INFO: {\"storage_info\": {\"total_blocks\":20, "
		"\"block_size\":512junk, \"spec_version\":ae80}}",
		"INFO: {\"storage_info\": {\"total_blocks\":20, "
		"\"block_size\":-1, \"spec_version\":ae80}}",
		"INFO: {\"storage_info\": {\"total_blocks\":20, \"spec_version\":ae80}}",
		"INFO: {\"storage_info\": {\"total_blocks\":20, \"block_size\":512",
		"INFO: {\"storage_info\": {\"total_blocks\":",
	};
	struct firehose_getsize_args args = { .sector_size = 4096, .num_sectors = 10 };
	size_t i;
	(void)state;

	for (i = 0; i < ARRAY_SIZE(bad); i++) {
		getstorageinfo_log(bad[i], &args);
		assert_int_equal(args.num_sectors, 10);
		assert_int_equal(args.sector_size, 4096);
	}
}

int main(void)
{
	const struct CMUnitTest tests[] = {
		cmocka_unit_test(test_generic_ack_nak_log),
		cmocka_unit_test(test_generic_rawmode),
		cmocka_unit_test(test_generic_missing_value),
		cmocka_unit_test(test_configure_ack),
		cmocka_unit_test(test_configure_nak_negotiates),
		cmocka_unit_test(test_configure_ack_without_supported),
		cmocka_unit_test(test_configure_log_and_missing),
		cmocka_unit_test(test_getstorageinfo_json),
		cmocka_unit_test(test_getstorageinfo_invalid_json),
		cmocka_unit_test(test_getstorageinfo_invalid_json_scope),
		cmocka_unit_test(test_getstorageinfo_invalid_json_bad_numbers),
	};

	return cmocka_run_group_tests(tests, NULL, NULL);
}
