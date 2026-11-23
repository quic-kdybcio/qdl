/* SPDX-License-Identifier: BSD-3-Clause */
/*
 * Copyright (c) Qualcomm Technologies, Inc. and/or its subsidiaries.
 */
#ifndef __PATHBUF_H__
#define __PATHBUF_H__

/*
 * struct pathbuf and the qdl_pathbuf_*() helpers are implemented in Rust
 * (rust/qdl-rs/src/pathbuf.rs). Their declarations come from qdl_rs.h.
 */
#include <limits.h>

#include "qdl_rs.h"

_Static_assert(QDL_PATH_MAX == PATH_MAX, "Rust path buffer size must match PATH_MAX");

#endif
