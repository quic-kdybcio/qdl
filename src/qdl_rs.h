/* SPDX-License-Identifier: BSD-3-Clause */
/*
 * Copyright (c) Qualcomm Technologies, Inc. and/or its subsidiaries.
 */
#ifndef QDL_RS_H
#define QDL_RS_H

/*
 * C view of the #[export] functions in rust/qdl-rs and the types they use.
 * Written by hand: keep it in sync with the Rust sources. A change to a
 * signature or to the layout of struct pathbuf needs the matching change
 * on both sides.
 */

#include <stddef.h>

/* Must match QDL_PATH_MAX in rust/qdl-rs/src/pathbuf.rs. */
#if defined(_WIN32)
#define QDL_PATH_MAX 260
#elif defined(__APPLE__)
#define QDL_PATH_MAX 1024
#else
#define QDL_PATH_MAX 4096
#endif

struct pathbuf {
	char buf[QDL_PATH_MAX];
	size_t len;
};

void qdl_pathbuf_reset(struct pathbuf *path);
void qdl_pathbuf_dup(struct pathbuf *dst, const struct pathbuf *orig);
int qdl_pathbuf_push(struct pathbuf *path, const char *component);
const char *qdl_pathbuf_str(const struct pathbuf *path);
void qdl_pathbuf_dirname(struct pathbuf *path);

#endif
