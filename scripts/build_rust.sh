#!/bin/sh
# SPDX-License-Identifier: BSD-3-Clause

# Build the Rust half of qdl and copy the static library to the path meson
# asks for. Invoked from meson/meson.build.
#
# usage: build_rust.sh CARGO MANIFEST TARGET_DIR LIB_OUT

set -eu

cargo=$1
manifest=$2
target_dir=$3
lib_out=$4

"$cargo" build --release --locked \
	--manifest-path "$manifest" --target-dir "$target_dir"

cp "$target_dir/release/libqdl_rs.a" "$lib_out"
